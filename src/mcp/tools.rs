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
    /// Company-language search terms (e.g. ["Tesla","Robotaxi"] for TSLA) —
    /// high-impact accounts rarely write cashtags, so propose these alongside
    /// accounts. Multi-word phrases are fine (e.g. "General Motors").
    pub keywords: Option<Vec<String>>,
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
    let keywords = args.keywords.unwrap_or_default();
    let report = pulse_app::pulse(
        &args.ticker,
        &accounts,
        &keywords,
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

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum RiskDirectionArg {
    Long,
    Short,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RiskToolArgs {
    /// Ticker symbol, e.g. "NVDA".
    pub ticker: String,
    /// Per-trade risk budget in USD — the most a stop-out may lose.
    pub budget_usd: f64,
    /// Trade direction (default long).
    pub direction: Option<RiskDirectionArg>,
    /// Stop distance in ATR multiples (default 2.0, clamped 0.5-5).
    pub stop_multiple: Option<f64>,
    /// Entry price override (default: last close).
    pub entry: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct RiskOutput {
    pub summary: String,
    pub frame: crate::domain::risk::RiskFrame,
    pub framing: &'static str,
    pub disclaimer: &'static str,
}

pub async fn run_risk_frame(
    args: RiskToolArgs,
    bars: &dyn crate::domain::ports::bar_source::BarSource,
) -> Result<RiskOutput, DomainError> {
    use crate::domain::risk::Direction;
    let direction = match args.direction.unwrap_or(RiskDirectionArg::Long) {
        RiskDirectionArg::Long => Direction::Long,
        RiskDirectionArg::Short => Direction::Short,
    };
    let frame = crate::application::risk::risk_frame(
        &args.ticker,
        direction,
        args.budget_usd,
        args.stop_multiple,
        args.entry,
        bars,
        chrono::Utc::now(),
    )
    .await?;
    let summary = format!(
        "{} {:?} — entry {:.2} · stop {:.2} · {} shares · max loss ${:.2} (≤ ${:.2}) · 1R {:.2}",
        frame.ticker,
        frame.direction,
        frame.entry,
        frame.stop,
        frame.shares,
        frame.max_loss_usd,
        frame.budget_usd,
        frame.targets[0]
    );
    Ok(RiskOutput {
        summary,
        frame,
        framing: "risk_frame is a calculator, not advice — it never recommends taking a trade.",
        disclaimer: DISCLAIMER,
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DipScanArgs {
    /// Evaluate one symbol instead of scanning the losers universe — the
    /// "considering a trade" mode. The quality-floor gate is unverifiable for
    /// a single ticker, so the verdict caps at watch.
    pub ticker: Option<String>,
    /// Day losers to pull from the screener (1-100, default 100).
    pub count: Option<usize>,
    /// Floor+band survivors to deep-analyze (1-25, default 10).
    pub deep_n: Option<usize>,
    /// Worst eligible day change in percent (default -15).
    pub band_min: Option<f64>,
    /// Mildest eligible day change in percent (default -4).
    pub band_max: Option<f64>,
    /// Account equity in USD — enables the risk + margin sizing section.
    pub equity_usd: Option<f64>,
    /// Buying-power multiple (overnight Reg-T caps at 2; default 2).
    pub leverage: Option<f64>,
    /// Unlock 4x intraday buying power (NOT holdable overnight).
    pub intraday_bp: Option<bool>,
    /// Maintenance requirement fraction (default 0.25).
    pub maintenance: Option<f64>,
    /// Fraction of equity risked to the stop per position (default 0.01).
    pub risk_pct: Option<f64>,
    /// Minimum composite score for the score gate (default 65).
    pub score_min: Option<f64>,
    /// Quality floor: minimum share price (default 5).
    pub min_price: Option<f64>,
    /// Quality floor: minimum market cap in USD (default 500M).
    pub min_cap: Option<u64>,
    /// Quality floor: minimum 3-month average daily volume (default 1M shares).
    pub min_volume: Option<u64>,
    /// Quality floor: minimum days since listing (default 180).
    pub min_listed_days: Option<i64>,
    /// Skip the scan journal write (~/.openintel/dip_journal.jsonl).
    pub no_journal: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct DipOutput {
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<crate::application::dip::DipScanReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal: Option<crate::domain::dip::DipSignal>,
    pub framing: &'static str,
    pub disclaimer: &'static str,
}

fn dip_request_from(args: &DipScanArgs) -> crate::application::dip::DipScanRequest {
    use crate::application::dip::{default_journal_path, DipScanRequest};
    use crate::domain::dip::{DropBand, QualityFloor};
    use crate::domain::margin::MarginInputs;
    let base = DipScanRequest::default();
    let band = DropBand::default();
    let floor = QualityFloor::default();
    let margin_defaults = MarginInputs::default();
    DipScanRequest {
        count: args.count.unwrap_or(base.count),
        deep_n: args.deep_n.unwrap_or(base.deep_n),
        band: DropBand {
            min_pct: args.band_min.unwrap_or(band.min_pct),
            max_pct: args.band_max.unwrap_or(band.max_pct),
        },
        floor: QualityFloor {
            min_price: args.min_price.unwrap_or(floor.min_price),
            min_market_cap: args.min_cap.unwrap_or(floor.min_market_cap),
            min_avg_volume: args.min_volume.unwrap_or(floor.min_avg_volume),
            min_listed_days: args.min_listed_days.unwrap_or(floor.min_listed_days),
        },
        score_min: args.score_min.unwrap_or(base.score_min),
        margin: args.equity_usd.map(|equity_usd| MarginInputs {
            equity_usd,
            leverage: args.leverage.unwrap_or(margin_defaults.leverage),
            maintenance: args.maintenance.unwrap_or(margin_defaults.maintenance),
            risk_pct: args.risk_pct.unwrap_or(margin_defaults.risk_pct),
            intraday_bp: args.intraday_bp.unwrap_or(false),
        }),
        journal_path: (!args.no_journal.unwrap_or(false))
            .then(default_journal_path)
            .flatten(),
        ..base
    }
}

pub async fn run_dip_scan(
    args: DipScanArgs,
    movers: &dyn crate::domain::ports::movers_source::MoversSource,
    deps: &crate::application::dip::DipDeps<'_>,
) -> Result<DipOutput, DomainError> {
    use crate::domain::dip::Verdict;
    let req = dip_request_from(&args);
    let now = Utc::now();
    match &args.ticker {
        Some(ticker) => {
            let signal = crate::application::dip::dip_check(ticker, &req, deps, now).await?;
            let summary = format!(
                "{} — {:?} · score {:.0}/{:.0}",
                signal.ticker, signal.verdict, signal.score, signal.score_max
            );
            Ok(DipOutput {
                summary,
                report: None,
                signal: Some(signal),
                framing: crate::application::dip::FRAMING,
                disclaimer: DISCLAIMER,
            })
        }
        None => {
            let report = crate::application::dip::dip_scan(&req, movers, deps, now).await?;
            let count = |v: Verdict| {
                report
                    .candidates
                    .iter()
                    .filter(|c| c.signal.verdict == v)
                    .count()
            };
            let summary = match report.candidates.first() {
                None => format!(
                    "no setups — 0 of {} losers survived the floor+band (that's a normal result)",
                    report.universe_size
                ),
                Some(top) => format!(
                    "{} analyzed: {} high_confidence, {} watch, {} no_setup · top: {} {:.0}/{:.0} ({:?})",
                    report.candidates.len(),
                    count(Verdict::HighConfidence),
                    count(Verdict::Watch),
                    count(Verdict::NoSetup),
                    top.ticker,
                    top.signal.score,
                    top.signal.score_max,
                    top.signal.verdict
                ),
            };
            Ok(DipOutput {
                summary,
                report: Some(report),
                signal: None,
                framing: crate::application::dip::FRAMING,
                disclaimer: DISCLAIMER,
            })
        }
    }
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
                _k: &[String],
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
                keywords: None,
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

    #[tokio::test]
    async fn run_pulse_threads_keywords_to_feed_and_report() {
        use crate::domain::entities::pulse::{PulseFetch, PulsePost};
        use crate::domain::entities::social_post::PostText;
        use crate::domain::entities::ticker::Ticker;
        use crate::domain::ports::influencer_feed::InfluencerFeed;
        use async_trait::async_trait;

        struct KeywordSpy;
        #[async_trait]
        impl InfluencerFeed for KeywordSpy {
            async fn pulse(
                &self,
                _t: &Ticker,
                _a: &[String],
                keywords: &[String],
                _h: u32,
                _l: usize,
            ) -> Result<PulseFetch, DomainError> {
                assert_eq!(keywords, ["Tesla".to_string(), "Robotaxi".to_string()]);
                Ok(PulseFetch {
                    posts: vec![PulsePost {
                        id: "1".into(),
                        author: "elonmusk".into(),
                        text: PostText::parse("robotaxi launch").unwrap(),
                        created_at: chrono::Utc::now(),
                        engagement: 5,
                    }],
                    posts_returned: 1,
                })
            }
        }

        let out = run_pulse(
            PulseToolArgs {
                ticker: "TSLA".into(),
                accounts: Some(vec!["elonmusk".into()]),
                keywords: Some(vec!["Tesla".into(), "Robotaxi".into()]),
                hours_back: None,
                limit: None,
            },
            &KeywordSpy,
        )
        .await
        .unwrap();
        assert_eq!(
            out.report.keywords,
            vec!["Tesla".to_string(), "Robotaxi".to_string()]
        );
    }

    #[tokio::test]
    async fn run_risk_frame_summarizes_and_disclaims() {
        use crate::domain::ports::bar_source::BarSource;
        use crate::domain::values::bar::Bar;
        use async_trait::async_trait;

        struct FixedBars;
        #[async_trait]
        impl BarSource for FixedBars {
            async fn bars(
                &self,
                _t: &crate::domain::entities::ticker::Ticker,
            ) -> Result<Vec<Bar>, DomainError> {
                let date = chrono::NaiveDate::from_ymd_opt(2026, 7, 15).unwrap();
                let mut v = vec![Bar {
                    date,
                    open: 100.0,
                    high: 101.0,
                    low: 99.0,
                    close: 100.0,
                }];
                for _ in 0..15 {
                    v.push(Bar {
                        date,
                        open: 106.0,
                        high: 108.0,
                        low: 104.0,
                        close: 106.0,
                    });
                }
                Ok(v)
            }
        }

        let out = run_risk_frame(
            RiskToolArgs {
                ticker: "NVDA".into(),
                budget_usd: 200.0,
                direction: Some(RiskDirectionArg::Long),
                stop_multiple: Some(2.0),
                entry: None,
            },
            &FixedBars,
        )
        .await
        .unwrap();
        assert!(out.summary.contains("25 shares"));
        assert!(out.framing.contains("risk_frame is a calculator"));
        assert!(out.disclaimer.contains("Not financial advice"));
    }

    mod dip {
        use super::*;
        use crate::adapters::filings::mock_filings::MockFilingsSource;
        use crate::adapters::market::mock_movers::MockMoversSource;
        use crate::adapters::market::mock_news::MockNewsSource;
        use crate::application::dip::{DipDeps, SPX_PROXY};
        use crate::domain::ports::bar_source::BarSource;
        use crate::domain::values::bar::Bar;
        use crate::domain::values::mover::MoverRow;
        use async_trait::async_trait;
        use chrono::NaiveDate;
        use std::collections::HashMap;

        fn dip_bars(drop_close: f64) -> Vec<Bar> {
            let mut v: Vec<Bar> = (1..=30)
                .map(|i| {
                    let c = 110.0 - i as f64 * 0.5;
                    Bar {
                        date: NaiveDate::from_ymd_opt(2026, 7, i).unwrap(),
                        open: c,
                        high: c + 1.0,
                        low: c - 1.0,
                        close: c,
                    }
                })
                .collect();
            v.push(Bar {
                date: NaiveDate::from_ymd_opt(2026, 8, 14).unwrap(),
                open: drop_close + 6.0,
                high: drop_close + 0.5,
                low: drop_close - 7.0,
                close: drop_close,
            });
            v
        }

        struct MapBars(HashMap<String, Vec<Bar>>);

        #[async_trait]
        impl BarSource for MapBars {
            async fn bars(
                &self,
                t: &crate::domain::entities::ticker::Ticker,
            ) -> Result<Vec<Bar>, DomainError> {
                self.0.get(t.as_str()).cloned().ok_or(DomainError::NoData)
            }
        }

        fn bars_map() -> MapBars {
            let mut m = HashMap::new();
            m.insert("GOOD".to_string(), dip_bars(87.4));
            // flat index proxy
            let mut spy = dip_bars(95.0);
            for b in &mut spy {
                b.open = 500.0;
                b.high = 501.0;
                b.low = 499.0;
                b.close = 500.0;
            }
            spy.last_mut().unwrap().close = 499.0;
            m.insert(SPX_PROXY.to_string(), spy);
            MapBars(m)
        }

        fn args() -> DipScanArgs {
            DipScanArgs {
                ticker: None,
                count: None,
                deep_n: None,
                band_min: None,
                band_max: None,
                equity_usd: None,
                leverage: None,
                intraday_bp: None,
                maintenance: None,
                risk_pct: None,
                score_min: None,
                min_price: None,
                min_cap: None,
                min_volume: None,
                min_listed_days: None,
                no_journal: Some(true),
            }
        }

        #[tokio::test]
        async fn scan_mode_summarizes_and_frames() {
            let bars = bars_map();
            let news = MockNewsSource(Ok(vec![]));
            let filings = MockFilingsSource(Ok(vec![]));
            let social = fixture_social();
            let movers = MockMoversSource(vec![MoverRow {
                symbol: "GOOD".into(),
                change_pct: -8.0,
                price: 87.4,
                market_cap: Some(2_000_000_000),
                avg_volume_3mo: Some(5_000_000),
                day_volume: Some(4_000_000),
                exchange: "NYSE".into(),
                first_trade_ms: Some(0),
            }]);
            let deps = DipDeps {
                bars: &bars,
                news: &news,
                filings: &filings,
                social: &social,
                market: None,
            };
            let out = run_dip_scan(args(), &movers, &deps).await.unwrap();
            assert!(out.report.is_some());
            assert!(out.signal.is_none());
            assert!(out.summary.contains("GOOD"), "got {}", out.summary);
            assert!(out.framing.contains("setup conformance"));
            assert!(out.disclaimer.contains("Not financial advice"));
        }

        #[tokio::test]
        async fn single_ticker_mode_returns_signal() {
            let bars = bars_map();
            let news = MockNewsSource(Ok(vec![]));
            let filings = MockFilingsSource(Ok(vec![]));
            let social = fixture_social();
            let movers = MockMoversSource(vec![]);
            let deps = DipDeps {
                bars: &bars,
                news: &news,
                filings: &filings,
                social: &social,
                market: None,
            };
            let mut a = args();
            a.ticker = Some("GOOD".into());
            let out = run_dip_scan(a, &movers, &deps).await.unwrap();
            assert!(out.report.is_none());
            let signal = out.signal.unwrap();
            assert_eq!(signal.ticker, "GOOD");
            // single-ticker mode: floor unverifiable -> never high_confidence
            assert_ne!(signal.verdict, crate::domain::dip::Verdict::HighConfidence);
        }

        #[test]
        fn request_mapping_honors_overrides() {
            let mut a = args();
            a.equity_usd = Some(10_000.0);
            a.band_min = Some(-12.0);
            a.deep_n = Some(5);
            let req = dip_request_from(&a);
            assert_eq!(req.band.min_pct, -12.0);
            assert_eq!(req.deep_n, 5);
            assert!(req.journal_path.is_none()); // no_journal in fixture args
            assert_eq!(req.margin.unwrap().equity_usd, 10_000.0);
        }
    }
}
