use crate::application::{self, request::AnalysisRequest, DISCLAIMER};
use crate::config::settings::{AppConfig, OutputFormat};
use crate::domain::entities::speculation_report::SpeculationReport;
use crate::domain::error::DomainError;
use crate::domain::ports::market_data_source::MarketDataSource;

pub async fn analyze(
    config: &AppConfig,
    market_source: &dyn MarketDataSource,
) -> Result<(SpeculationReport, String), DomainError> {
    let req = AnalysisRequest {
        ticker: config.ticker.clone(),
        enabled_sources: config.enabled_sources.clone(),
        market_enabled: config.market_enabled,
        limit: config.limit,
        engine: config.engine.clone(),
    };
    let report = application::analyze(&req, market_source).await?;
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
    .unwrap_or_else(|e| {
        serde_json::json!({ "error": format!("serialization failed: {e}") }).to_string()
    })
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
            let rvol_str = m
                .rvol
                .map(|r| format!("{r:.2}x"))
                .unwrap_or_else(|| "n/a".to_string());
            let _ = writeln!(out, "\nMARKET");
            let _ = writeln!(
                out,
                "  last: {:.2}  change: {:+.2}%  rvol: {}",
                m.last_price, m.pct_change, rvol_str
            );
        }
        None => {
            let failed = report
                .fusion
                .notes
                .iter()
                .any(|n| n.contains("market source failed"));
            let label = if failed {
                "(unavailable — fetch failed; see notes)"
            } else {
                "(disabled)"
            };
            let _ = writeln!(out, "\nMARKET\n  {label}");
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
    use crate::adapters::market::mock_market::MockMarketSource;
    use crate::config::settings::{AppConfig, OutputFormat};
    use crate::domain::values::speculation::Alignment;

    fn config(no_market: bool, format: OutputFormat) -> AppConfig {
        AppConfig::new("AAPL".into(), false, false, false, no_market, 50, format)
    }

    #[tokio::test]
    async fn full_run_confirms_bullish_with_market() {
        let (report, rendered) = analyze(&config(false, OutputFormat::Json), &MockMarketSource)
            .await
            .unwrap();
        assert!(report.market.is_some());
        assert_eq!(report.fusion.alignment, Alignment::ConfirmingBullish);
        assert!(rendered.contains("Not financial advice"));
        assert!(rendered.contains("speculation_index"));
    }

    #[tokio::test]
    async fn no_market_run_is_quiet() {
        let (report, _) = analyze(&config(true, OutputFormat::Table), &MockMarketSource)
            .await
            .unwrap();
        assert!(report.market.is_none());
        assert_eq!(report.fusion.alignment, Alignment::Quiet);
    }

    #[tokio::test]
    async fn table_output_has_sections_and_disclaimer() {
        let (_, rendered) = analyze(&config(false, OutputFormat::Table), &MockMarketSource)
            .await
            .unwrap();
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
        assert!(analyze(&cfg, &MockMarketSource).await.is_err());
    }
}
