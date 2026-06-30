use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::application::{self, request::AnalysisRequest, DISCLAIMER};
use crate::domain::engine::config::EngineConfig;
use crate::domain::entities::speculation_report::SpeculationReport;
use crate::domain::error::DomainError;
use crate::domain::ports::market_data_source::MarketDataSource;
use crate::domain::values::source_kind::SourceKind;

#[derive(Debug, Serialize)]
pub struct SourcesOutput {
    pub social: Vec<String>,
    pub market: Vec<String>,
}

/// Derived from `SourceKind::ALL` (one source of truth) + the market adapter's name.
pub fn run_list_sources() -> SourcesOutput {
    SourcesOutput {
        social: SourceKind::ALL
            .iter()
            .map(|s| s.as_str().to_string())
            .collect(),
        market: vec![crate::adapters::market::mock_market::MockMarketSource
            .name()
            .to_string()],
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AnalyzeArgs {
    /// Ticker symbol, e.g. "AAPL".
    pub ticker: String,
    /// Enable the Reddit source (if no source flags are set, all are enabled).
    pub enable_reddit: Option<bool>,
    pub enable_x: Option<bool>,
    pub enable_bluesky: Option<bool>,
    /// Skip the market snapshot (social-only report).
    pub no_market: Option<bool>,
    /// Posts to fetch per source (default 50).
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct AnalyzeOutput {
    pub summary: String,
    pub report: SpeculationReport,
    pub disclaimer: &'static str,
}

/// Build an `AnalysisRequest` from tool options. Shared by all analysis tools.
pub(crate) fn request_from(
    ticker: String,
    enable_reddit: Option<bool>,
    enable_x: Option<bool>,
    enable_bluesky: Option<bool>,
    no_market: Option<bool>,
    limit: Option<usize>,
) -> AnalysisRequest {
    let mut enabled = Vec::new();
    if enable_reddit.unwrap_or(false) {
        enabled.push(SourceKind::Reddit);
    }
    if enable_x.unwrap_or(false) {
        enabled.push(SourceKind::X);
    }
    if enable_bluesky.unwrap_or(false) {
        enabled.push(SourceKind::Bluesky);
    }
    if enabled.is_empty() {
        enabled = SourceKind::ALL.to_vec();
    }
    AnalysisRequest {
        ticker,
        enabled_sources: enabled,
        market_enabled: !no_market.unwrap_or(false),
        limit: limit.unwrap_or(50),
        engine: EngineConfig::default(),
    }
}

/// One-line human gloss for the text-content side of a tool result.
pub(crate) fn summarize(report: &SpeculationReport) -> String {
    format!(
        "{} — {:?} · crowding {:.0}% · {} mentions ({:?})",
        report.ticker.as_str(),
        report.fusion.alignment,
        report.fusion.crowding * 100.0,
        report.social.total_mentions,
        report.social_confidence,
    )
}

pub async fn run_analyze(args: AnalyzeArgs) -> Result<AnalyzeOutput, DomainError> {
    let req = request_from(
        args.ticker,
        args.enable_reddit,
        args.enable_x,
        args.enable_bluesky,
        args.no_market,
        args.limit,
    );
    let report = application::analyze(&req).await?;
    Ok(AnalyzeOutput {
        summary: summarize(&report),
        report,
        disclaimer: DISCLAIMER,
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScanArgs {
    /// Ticker symbols to analyze concurrently.
    pub tickers: Vec<String>,
    pub enable_reddit: Option<bool>,
    pub enable_x: Option<bool>,
    pub enable_bluesky: Option<bool>,
    /// Skip the market snapshot (social-only report).
    pub no_market: Option<bool>,
    /// Posts to fetch per source (default 50).
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct ScanEntry {
    pub ticker: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<SpeculationReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ScanOutput {
    pub entries: Vec<ScanEntry>,
    pub disclaimer: &'static str,
}

pub async fn run_scan(args: ScanArgs) -> ScanOutput {
    let ScanArgs {
        tickers,
        enable_reddit,
        enable_x,
        enable_bluesky,
        no_market,
        limit,
    } = args;
    let futures = tickers.into_iter().map(|t| async move {
        let req = request_from(
            t.clone(),
            enable_reddit,
            enable_x,
            enable_bluesky,
            no_market,
            limit,
        );
        match application::analyze(&req).await {
            Ok(report) => ScanEntry {
                ticker: t,
                report: Some(report),
                error: None,
            },
            Err(e) => ScanEntry {
                ticker: t,
                report: None,
                error: Some(e.to_string()),
            },
        }
    });
    let entries = futures::future::join_all(futures).await;
    ScanOutput {
        entries,
        disclaimer: DISCLAIMER,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_sources_reports_all_adapters() {
        let out = run_list_sources();
        assert_eq!(out.social, vec!["reddit", "x", "bluesky"]);
        assert_eq!(out.market, vec!["mock-market"]);
    }

    #[tokio::test]
    async fn run_analyze_returns_confirming_bullish_report() {
        let out = run_analyze(AnalyzeArgs {
            ticker: "AAPL".into(),
            enable_reddit: None,
            enable_x: None,
            enable_bluesky: None,
            no_market: None,
            limit: None,
        })
        .await
        .unwrap();
        assert!(out.summary.contains("ConfirmingBullish"));
        assert_eq!(out.report.social.total_mentions, 10);
        assert!(out.disclaimer.contains("Not financial advice"));
    }

    #[tokio::test]
    async fn run_analyze_rejects_bad_ticker() {
        let args = AnalyzeArgs {
            ticker: "$$$".into(),
            enable_reddit: None,
            enable_x: None,
            enable_bluesky: None,
            no_market: None,
            limit: None,
        };
        assert!(run_analyze(args).await.is_err());
    }

    #[tokio::test]
    async fn run_scan_handles_mixed_batch() {
        let out = run_scan(ScanArgs {
            tickers: vec!["AAPL".into(), "$$$".into()],
            enable_reddit: None,
            enable_x: None,
            enable_bluesky: None,
            no_market: None,
            limit: None,
        })
        .await;
        assert_eq!(out.entries.len(), 2);
        assert!(out.entries[0].report.is_some() && out.entries[0].error.is_none());
        assert!(out.entries[1].report.is_none() && out.entries[1].error.is_some());
        assert!(out.disclaimer.contains("Not financial advice"));
    }

    #[tokio::test]
    async fn run_scan_empty_list_is_empty() {
        let out = run_scan(ScanArgs {
            tickers: vec![],
            enable_reddit: None,
            enable_x: None,
            enable_bluesky: None,
            no_market: None,
            limit: None,
        })
        .await;
        assert!(out.entries.is_empty());
    }
}
