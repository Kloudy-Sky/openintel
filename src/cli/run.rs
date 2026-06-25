use chrono::Utc;
use futures::future::join_all;

use crate::adapters::analyzer::lexicon::LexiconAnalyzer;
use crate::adapters::market::mock_market::MockMarketSource;
use crate::adapters::sources::mock_bluesky::MockBlueskySource;
use crate::adapters::sources::mock_reddit::MockRedditSource;
use crate::adapters::sources::mock_x::MockXSource;
use crate::config::settings::{AppConfig, OutputFormat};
use crate::domain::engine::speculation_engine::SpeculationEngine;
use crate::domain::entities::market_snapshot::MarketSnapshot;
use crate::domain::entities::social_post::SocialPost;
use crate::domain::entities::speculation_report::SpeculationReport;
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::ports::market_data_source::MarketDataSource;
use crate::domain::ports::post_analyzer::PostAnalyzer;
use crate::domain::ports::social_data_source::SocialDataSource;
use crate::domain::values::source_kind::SourceKind;

pub const DISCLAIMER: &str = "Not financial advice. OpenIntel is a research/screening tool; \
markets are risky and social data is easily manipulated. Do your own diligence.";

fn build_sources(config: &AppConfig) -> Vec<Box<dyn SocialDataSource>> {
    config
        .enabled_sources
        .iter()
        .map(|kind| -> Box<dyn SocialDataSource> {
            match kind {
                SourceKind::Reddit => Box::new(MockRedditSource),
                SourceKind::X => Box::new(MockXSource),
                SourceKind::Bluesky => Box::new(MockBlueskySource),
            }
        })
        .collect()
}

pub async fn analyze(config: &AppConfig) -> Result<(SpeculationReport, String), DomainError> {
    let ticker = Ticker::parse(&config.ticker)?;
    let sources = build_sources(config);

    // Concurrent social fetch; a single source failing is non-fatal.
    let fetches = sources.iter().map(|source| {
        let ticker = ticker.clone();
        async move { (source.kind(), source.fetch(&ticker, config.limit).await) }
    });
    let results = join_all(fetches).await;

    let mut posts: Vec<SocialPost> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    for (kind, result) in results {
        match result {
            Ok(mut fetched) => posts.append(&mut fetched),
            Err(e) => notes.push(format!("source {} failed: {e}", kind.as_str())),
        }
    }

    let market: Option<MarketSnapshot> = if config.market_enabled {
        match MockMarketSource.snapshot(&ticker).await {
            Ok(snapshot) => Some(snapshot),
            Err(e) => {
                notes.push(format!("market source failed: {e}"));
                None
            }
        }
    } else {
        None
    };

    if posts.is_empty() && market.is_none() {
        return Err(DomainError::NoData);
    }

    let analyzer = LexiconAnalyzer::new();
    let signals = analyzer.analyze(&posts).await?;

    let now = Utc::now();
    let mut report = SpeculationEngine::aggregate(
        &ticker,
        &posts,
        &signals,
        market.as_ref(),
        now,
        &config.engine,
    )?;

    // Prepend orchestration notes (source/market failures) ahead of engine notes.
    let engine_notes = std::mem::take(&mut report.fusion.notes);
    report.fusion.notes = notes.into_iter().chain(engine_notes).collect();

    let rendered = render(&report, config.format);
    Ok((report, rendered))
}

fn render(report: &SpeculationReport, format: OutputFormat) -> String {
    match format {
        OutputFormat::Json => render_json(report),
        OutputFormat::Table => render_table(report),
    }
}

fn render_json(report: &SpeculationReport) -> String {
    #[derive(serde::Serialize)]
    struct Envelope<'a> {
        #[serde(flatten)]
        report: &'a SpeculationReport,
        disclaimer: &'static str,
    }
    serde_json::to_string_pretty(&Envelope {
        report,
        disclaimer: DISCLAIMER,
    })
    .unwrap_or_else(|e| format!("{{\"error\":\"serialization failed: {e}\"}}"))
}

fn render_table(report: &SpeculationReport) -> String {
    use std::fmt::Write;

    let mut out = String::new();
    let s = &report.social;
    let _ = writeln!(out, "=== OpenIntel — {} ===", report.ticker.as_str());
    let _ = writeln!(out, "generated: {}", report.generated_at.to_rfc3339());
    let _ = writeln!(
        out,
        "confidence (social sample): {:?}",
        report.social_confidence
    );
    let _ = writeln!(out, "\nSOCIAL");
    let _ = writeln!(
        out,
        "  mentions: {} (bull {} / bear {} / neutral {})",
        s.total_mentions, s.bullish, s.bearish, s.neutral
    );
    let _ = writeln!(out, "  net sentiment: {:+.2}", s.net_sentiment.value());
    let _ = writeln!(
        out,
        "  speculation index: {:.0}%",
        s.speculation_index.value() * 100.0
    );
    match s.bull_bear_ratio {
        Some(r) => {
            let _ = writeln!(out, "  bull/bear ratio: {r:.2}");
        }
        None => {
            let _ = writeln!(out, "  bull/bear ratio: n/a (no bearish posts)");
        }
    }

    match &report.market {
        Some(m) => {
            let _ = writeln!(out, "\nMARKET");
            let _ = writeln!(
                out,
                "  last: {:.2}  change: {:+.2}%  rvol: {:.2}x",
                m.last_price, m.pct_change, m.rvol
            );
        }
        None => {
            let _ = writeln!(out, "\nMARKET\n  (disabled)");
        }
    }

    let _ = writeln!(out, "\nFUSION");
    let _ = writeln!(out, "  alignment: {:?}", report.fusion.alignment);
    let _ = writeln!(out, "  crowding: {:.0}%", report.fusion.crowding * 100.0);
    for note in &report.fusion.notes {
        let _ = writeln!(out, "  note: {note}");
    }

    let _ = writeln!(out, "\n{DISCLAIMER}");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::settings::{AppConfig, OutputFormat};
    use crate::domain::values::speculation::Alignment;

    fn config(no_market: bool, format: OutputFormat) -> AppConfig {
        AppConfig::new("AAPL".into(), false, false, false, no_market, 50, format)
    }

    #[tokio::test]
    async fn full_run_confirms_bullish_with_market() {
        let (report, rendered) = analyze(&config(false, OutputFormat::Json)).await.unwrap();
        assert!(report.market.is_some());
        assert_eq!(report.fusion.alignment, Alignment::ConfirmingBullish);
        assert!(rendered.contains("Not financial advice"));
        assert!(rendered.contains("speculation_index"));
    }

    #[tokio::test]
    async fn no_market_run_is_quiet() {
        let (report, _) = analyze(&config(true, OutputFormat::Table)).await.unwrap();
        assert!(report.market.is_none());
        assert_eq!(report.fusion.alignment, Alignment::Quiet);
    }

    #[tokio::test]
    async fn table_output_has_sections_and_disclaimer() {
        let (_, rendered) = analyze(&config(false, OutputFormat::Table)).await.unwrap();
        assert!(rendered.contains("SOCIAL"));
        assert!(rendered.contains("MARKET"));
        assert!(rendered.contains("FUSION"));
        assert!(rendered.contains("Not financial advice"));
    }

    #[tokio::test]
    async fn invalid_ticker_errors() {
        let cfg = AppConfig::new(
            "$$$".into(),
            false,
            false,
            false,
            false,
            50,
            OutputFormat::Table,
        );
        assert!(analyze(&cfg).await.is_err());
    }
}
