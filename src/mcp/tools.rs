use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::application::{self, pulse as pulse_app, request::AnalysisRequest, DISCLAIMER};
use crate::domain::engine::config::EngineConfig;
use crate::domain::entities::pulse::PulseReport;
use crate::domain::entities::speculation_report::SpeculationReport;
use crate::domain::error::DomainError;
use crate::domain::ports::influencer_feed::InfluencerFeed;
use crate::domain::ports::market_data_source::MarketDataSource;
use crate::domain::ports::social_data_source::SocialDataSource;
use crate::domain::values::source_kind::SourceKind;
use crate::domain::values::speculation::Alignment;
use chrono::Utc;

#[derive(Debug, Serialize)]
pub struct SourcesOutput {
    pub social: Vec<String>,
    pub market: Vec<String>,
}

/// Report the actually-wired data sources so an agent can see whether an
/// optional source (e.g. Reddit, which needs OAuth credentials) is live —
/// `social` reflects the injected list, not the full `SourceKind::ALL` set.
pub fn run_list_sources(
    social_sources: &[Box<dyn SocialDataSource>],
    market_source: &dyn MarketDataSource,
) -> SourcesOutput {
    SourcesOutput {
        social: social_sources
            .iter()
            .map(|s| s.kind().as_str().to_string())
            .collect(),
        market: vec![market_source.name().to_string()],
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AnalyzeArgs {
    /// Ticker symbol, e.g. "AAPL".
    pub ticker: String,
    /// Enable the Reddit source (if no source flags are set, all are enabled).
    pub enable_reddit: Option<bool>,
    /// Enable the Bluesky source (if no source flags are set, all are enabled).
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
    enable_bluesky: Option<bool>,
    no_market: Option<bool>,
    limit: Option<usize>,
) -> AnalysisRequest {
    let mut enabled = Vec::new();
    if enable_reddit.unwrap_or(false) {
        enabled.push(SourceKind::Reddit);
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

pub async fn run_analyze(
    args: AnalyzeArgs,
    social_sources: &[Box<dyn SocialDataSource>],
    market_source: &dyn MarketDataSource,
) -> Result<AnalyzeOutput, DomainError> {
    let req = request_from(
        args.ticker,
        args.enable_reddit,
        args.enable_bluesky,
        args.no_market,
        args.limit,
    );
    let report = application::analyze(&req, social_sources, Some(market_source)).await?;
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
    /// Enable the Reddit source (if no source flags are set, all are enabled).
    pub enable_reddit: Option<bool>,
    /// Enable the Bluesky source (if no source flags are set, all are enabled).
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

pub async fn run_scan(
    args: ScanArgs,
    social_sources: &[Box<dyn SocialDataSource>],
    market_source: &dyn MarketDataSource,
) -> ScanOutput {
    let ScanArgs {
        tickers,
        enable_reddit,
        enable_bluesky,
        no_market,
        limit,
    } = args;
    let futures = tickers.into_iter().map(|t| async move {
        let req = request_from(t.clone(), enable_reddit, enable_bluesky, no_market, limit);
        match application::analyze(&req, social_sources, Some(market_source)).await {
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

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RankBy {
    /// Blended crowding score (default).
    #[default]
    Crowding,
    SpeculationIndex,
    NetSentiment,
    /// Diverging tickers first, then by crowding.
    Divergence,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CompareArgs {
    pub tickers: Vec<String>,
    #[serde(default)]
    pub rank_by: RankBy,
    /// Enable the Reddit source (if no source flags are set, all are enabled).
    pub enable_reddit: Option<bool>,
    /// Enable the Bluesky source (if no source flags are set, all are enabled).
    pub enable_bluesky: Option<bool>,
    pub no_market: Option<bool>,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct RankedEntry {
    pub ticker: String,
    pub rank_metric: f64,
    pub report: SpeculationReport,
}

#[derive(Debug, Serialize)]
pub struct CompareError {
    pub ticker: String,
    pub error: String,
}

#[derive(Debug, Serialize)]
pub struct CompareOutput {
    pub rank_by: RankBy,
    pub ranked: Vec<RankedEntry>,
    pub errors: Vec<CompareError>,
    pub disclaimer: &'static str,
}

fn rank_metric(report: &SpeculationReport, rank_by: RankBy) -> f64 {
    match rank_by {
        // `divergence` ranks categorically (diverging first) then by crowding,
        // so its numeric metric is crowding.
        RankBy::Crowding | RankBy::Divergence => report.fusion.crowding,
        RankBy::SpeculationIndex => report.social.speculation_index.value(),
        RankBy::NetSentiment => report.social.net_sentiment.value(),
    }
}

pub(crate) fn sort_ranked(ranked: &mut [RankedEntry], rank_by: RankBy) {
    ranked.sort_by(|a, b| {
        if matches!(rank_by, RankBy::Divergence) {
            let a_div = matches!(a.report.fusion.alignment, Alignment::Diverging);
            let b_div = matches!(b.report.fusion.alignment, Alignment::Diverging);
            b_div.cmp(&a_div).then_with(|| {
                b.rank_metric
                    .partial_cmp(&a.rank_metric)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        } else {
            b.rank_metric
                .partial_cmp(&a.rank_metric)
                .unwrap_or(std::cmp::Ordering::Equal)
        }
    });
}

pub async fn run_compare(
    args: CompareArgs,
    social_sources: &[Box<dyn SocialDataSource>],
    market_source: &dyn MarketDataSource,
) -> CompareOutput {
    let CompareArgs {
        tickers,
        rank_by,
        enable_reddit,
        enable_bluesky,
        no_market,
        limit,
    } = args;
    let futures = tickers.into_iter().map(|t| async move {
        let req = request_from(t.clone(), enable_reddit, enable_bluesky, no_market, limit);
        (
            t,
            application::analyze(&req, social_sources, Some(market_source)).await,
        )
    });
    let results = futures::future::join_all(futures).await;

    let mut ranked: Vec<RankedEntry> = Vec::new();
    let mut errors: Vec<CompareError> = Vec::new();
    for (ticker, res) in results {
        match res {
            Ok(report) => {
                let metric = rank_metric(&report, rank_by);
                ranked.push(RankedEntry {
                    ticker,
                    rank_metric: metric,
                    report,
                });
            }
            Err(e) => errors.push(CompareError {
                ticker,
                error: e.to_string(),
            }),
        }
    }
    sort_ranked(&mut ranked, rank_by);

    CompareOutput {
        rank_by,
        ranked,
        errors,
        disclaimer: DISCLAIMER,
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PulseToolArgs {
    /// Ticker symbol, e.g. "NVDA".
    pub ticker: String,
    /// X handles to listen to (no @). Curate per ticker: CEO/founder, major
    /// holders or activist funds, sector journalists, macro figures. Omit only
    /// if the user asked for the default macro list.
    pub accounts: Option<Vec<String>>,
    /// Lookback window in hours (default 24, max 167).
    pub hours_back: Option<u32>,
    /// Max posts to read — each read costs ~$0.005 (default 20, max 100).
    /// X bills a minimum of 10 reads per call.
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct PulseOutput {
    pub summary: String,
    pub report: PulseReport,
    pub disclaimer: &'static str,
}

pub async fn run_pulse(
    args: PulseToolArgs,
    feed: &dyn InfluencerFeed,
) -> Result<PulseOutput, DomainError> {
    let accounts = args.accounts.unwrap_or_default();
    let report = pulse_app::pulse(
        &args.ticker,
        &accounts,
        args.hours_back.unwrap_or(24),
        args.limit.unwrap_or(20),
        feed,
        Utc::now(),
    )
    .await?;
    let summary = format!(
        "{} — ⚡ {} high-impact post(s) in last {}h from {} account(s) · {} posts read ≈ ${:.2}",
        report.ticker,
        report.posts.len(),
        report.hours_back,
        report.accounts.len(),
        report.posts_read,
        report.estimated_cost_usd
    );
    Ok(PulseOutput {
        summary,
        report,
        disclaimer: DISCLAIMER,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::market::mock_market::MockMarketSource;
    use crate::adapters::sources::test_fixtures::fixture_social;

    #[test]
    fn list_sources_reports_all_adapters() {
        let out = run_list_sources(&fixture_social(), &MockMarketSource);
        assert_eq!(out.social, vec!["reddit", "bluesky"]);
        assert_eq!(out.market, vec!["mock-market"]);
    }

    #[tokio::test]
    async fn run_analyze_returns_confirming_bullish_report() {
        let out = run_analyze(
            AnalyzeArgs {
                ticker: "AAPL".into(),
                enable_reddit: None,
                enable_bluesky: None,
                no_market: None,
                limit: None,
            },
            &fixture_social(),
            &MockMarketSource,
        )
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
            enable_bluesky: None,
            no_market: None,
            limit: None,
        };
        assert!(run_analyze(args, &fixture_social(), &MockMarketSource)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn run_scan_handles_mixed_batch() {
        let out = run_scan(
            ScanArgs {
                tickers: vec!["AAPL".into(), "$$$".into()],
                enable_reddit: None,
                enable_bluesky: None,
                no_market: None,
                limit: None,
            },
            &fixture_social(),
            &MockMarketSource,
        )
        .await;
        assert_eq!(out.entries.len(), 2);
        assert!(out.entries[0].report.is_some() && out.entries[0].error.is_none());
        assert!(out.entries[1].report.is_none() && out.entries[1].error.is_some());
        assert!(out.disclaimer.contains("Not financial advice"));
    }

    #[tokio::test]
    async fn run_scan_empty_list_is_empty() {
        let out = run_scan(
            ScanArgs {
                tickers: vec![],
                enable_reddit: None,
                enable_bluesky: None,
                no_market: None,
                limit: None,
            },
            &fixture_social(),
            &MockMarketSource,
        )
        .await;
        assert!(out.entries.is_empty());
    }

    #[tokio::test]
    async fn sort_ranked_orders_by_crowding_desc() {
        use crate::domain::engine::config::EngineConfig;
        use crate::domain::engine::speculation_engine::SpeculationEngine;
        use crate::domain::entities::social_post::{PostText, SocialPost};
        use crate::domain::entities::ticker::Ticker;
        use crate::domain::values::polarity::Polarity;
        use crate::domain::values::post_signal::PostSignal;
        use chrono::{TimeZone, Utc};

        let t = Ticker::parse("AAPL").unwrap();
        let now = Utc.with_ymd_and_hms(2026, 6, 29, 0, 0, 0).unwrap();
        let post = SocialPost {
            id: "1".into(),
            source: SourceKind::Reddit,
            author: "a".into(),
            text: PostText::parse("x").unwrap(),
            created_at: now,
            engagement: 0,
        };
        // high crowding: speculative post; low crowding: non-speculative.
        let hi = SpeculationEngine::aggregate(
            &t,
            std::slice::from_ref(&post),
            &[PostSignal {
                polarity: Polarity::new(0.0),
                speculative: true,
            }],
            None,
            now,
            &EngineConfig::default(),
        )
        .unwrap();
        let lo = SpeculationEngine::aggregate(
            &t,
            std::slice::from_ref(&post),
            &[PostSignal {
                polarity: Polarity::new(0.0),
                speculative: false,
            }],
            None,
            now,
            &EngineConfig::default(),
        )
        .unwrap();
        assert!(hi.fusion.crowding > lo.fusion.crowding);

        let mut ranked = vec![
            RankedEntry {
                ticker: "LO".into(),
                rank_metric: lo.fusion.crowding,
                report: lo,
            },
            RankedEntry {
                ticker: "HI".into(),
                rank_metric: hi.fusion.crowding,
                report: hi,
            },
        ];
        sort_ranked(&mut ranked, RankBy::Crowding);
        assert_eq!(ranked[0].ticker, "HI");
        assert_eq!(ranked[1].ticker, "LO");
    }

    #[tokio::test]
    async fn run_compare_partitions_valid_and_invalid() {
        let out = run_compare(
            CompareArgs {
                tickers: vec!["AAPL".into(), "$$$".into()],
                rank_by: RankBy::Crowding,
                enable_reddit: None,
                enable_bluesky: None,
                no_market: None,
                limit: None,
            },
            &fixture_social(),
            &MockMarketSource,
        )
        .await;
        assert_eq!(out.ranked.len(), 1);
        assert_eq!(out.errors.len(), 1);
        assert_eq!(out.errors[0].ticker, "$$$");
        assert!(out.ranked[0].rank_metric.is_finite());
    }

    #[tokio::test]
    async fn run_pulse_summarizes_and_costs() {
        use crate::domain::entities::pulse::{PulseFetch, PulsePost};
        use crate::domain::entities::social_post::PostText;
        use crate::domain::entities::ticker::Ticker;
        use crate::domain::ports::influencer_feed::InfluencerFeed;
        use async_trait::async_trait;

        struct OnePost;
        #[async_trait]
        impl InfluencerFeed for OnePost {
            async fn pulse(
                &self,
                _t: &Ticker,
                _a: &[String],
                _h: u32,
                _l: usize,
            ) -> Result<PulseFetch, DomainError> {
                Ok(PulseFetch {
                    posts: vec![PulsePost {
                        id: "1".into(),
                        author: "jensenhuang".into(),
                        text: PostText::parse("shipping").unwrap(),
                        created_at: chrono::Utc::now(),
                        engagement: 5,
                    }],
                    posts_returned: 1,
                })
            }
        }

        let out = run_pulse(
            PulseToolArgs {
                ticker: "NVDA".into(),
                accounts: Some(vec!["@jensenhuang".into()]),
                hours_back: None,
                limit: None,
            },
            &OnePost,
        )
        .await
        .unwrap();
        assert!(out.summary.contains("⚡ 1 high-impact post(s)"));
        assert_eq!(out.report.accounts, vec!["jensenhuang"]); // @-stripped
        assert!(out.disclaimer.contains("Not financial advice"));
    }
}
