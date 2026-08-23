//! IO edge for `discover` movers mode: pull the predefined screens, floor the
//! merged universe, and annotate a bounded slice with tape context, period
//! extremes, catalyst gates, and social attention. Evidence only — no
//! ranking, no verdicts; losers point at dip_scan for the gated verdict.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use futures::StreamExt;
use serde::Serialize;

use crate::domain::dip::{
    apply_floor, filing_gate, headline_gate, rsi_cutler, sma, GateEvidence, GateStatus,
    QualityFloor, SentimentSummary,
};
use crate::domain::discover::{allocate_deep, period_extremes, PeriodExtremes};
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::ports::bar_source::BarSource;
use crate::domain::ports::filings_source::FilingsSource;
use crate::domain::ports::movers_source::MoversSource;
use crate::domain::ports::news_source::NewsSource;
use crate::domain::ports::social_data_source::SocialDataSource;
use crate::domain::risk::{atr, ATR_PERIOD};
use crate::domain::values::mover::{MoverRow, ScreenKind};

const CONCURRENCY: usize = 4;
const HEADLINE_COUNT: usize = 20;
const SMA_PERIOD: usize = 20;
const RSI_PERIOD: usize = 14;
const DAYS_3MO: usize = 63;
const DAYS_1Y: usize = 252;

pub const FRAMING: &str = "discover surfaces candidates with evidence — it never ranks, \
     never predicts, and never recommends. For gated dip verdicts on losers, run dip_scan.";

fn fail(message: impl Into<String>) -> DomainError {
    DomainError::SourceFailure {
        name: "discover".into(),
        message: message.into(),
    }
}

pub struct DiscoverDeps<'a> {
    pub movers: &'a dyn MoversSource,
    pub bars: &'a dyn BarSource,
    pub news: &'a dyn NewsSource,
    pub filings: &'a dyn FilingsSource,
    pub social: &'a [Box<dyn SocialDataSource>],
}

#[derive(Debug, Clone)]
pub struct DiscoverRequest {
    pub screens: Vec<ScreenKind>,
    /// Rows pulled per screen (1-100).
    pub count: usize,
    /// Total candidates deep-annotated, spread round-robin across screens.
    pub deep: usize,
    pub floor: QualityFloor,
}

impl Default for DiscoverRequest {
    fn default() -> Self {
        Self {
            screens: vec![
                ScreenKind::DayGainers,
                ScreenKind::DayLosers,
                ScreenKind::MostActives,
            ],
            count: 25,
            deep: 9,
            floor: QualityFloor::default(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ScreenSummary {
    pub screen: ScreenKind,
    pub universe: usize,
    pub floor_rejects: usize,
    pub annotated: usize,
}

#[derive(Debug, Serialize)]
pub struct CatalystCheck {
    pub filings: GateStatus,
    pub headlines: GateStatus,
    pub evidence: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Candidate {
    pub ticker: String,
    /// Which screens surfaced it (a symbol can appear in more than one).
    pub screens: Vec<ScreenKind>,
    pub change_pct: f64,
    pub price: f64,
    /// Day volume over 3-month average.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rvol: Option<f64>,
    /// (SMA20 − last) / ATR — positive = stretched below its mean.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stretch_atr: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rsi14: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extremes_3mo: Option<PeriodExtremes>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extremes_1y: Option<PeriodExtremes>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalyst: Option<CatalystCheck>,
    /// Social attention where sources are configured — crowding context, not
    /// a signal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sentiment: Option<SentimentSummary>,
    pub notes: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct DiscoverReport {
    pub generated_at: DateTime<Utc>,
    pub screens: Vec<ScreenSummary>,
    pub candidates: Vec<Candidate>,
    pub errors: Vec<String>,
    pub notes: Vec<String>,
}

async fn annotate(row: &MoverRow, screens: Vec<ScreenKind>, deps: &DiscoverDeps<'_>) -> Candidate {
    let mut notes = Vec::new();
    let ticker = match Ticker::parse(&row.symbol) {
        Ok(t) => t,
        Err(e) => {
            return Candidate {
                ticker: row.symbol.clone(),
                screens,
                change_pct: row.change_pct,
                price: row.price,
                rvol: rvol_of(row),
                stretch_atr: None,
                rsi14: None,
                extremes_3mo: None,
                extremes_1y: None,
                catalyst: None,
                sentiment: None,
                notes: vec![format!("unparseable symbol: {e}")],
            }
        }
    };

    let (mut stretch, mut rsi14, mut e3, mut e1y) = (None, None, None, None);
    match deps.bars.bars_long(&ticker).await {
        Err(e) => notes.push(format!("bars unavailable: {e}")),
        Ok(bars) => {
            let closes: Vec<f64> = bars.iter().map(|b| b.close).collect();
            match atr(&bars, ATR_PERIOD).filter(|a| *a > 0.0) {
                None => notes.push("not enough history for ATR — levels unscored".into()),
                Some(atr14) => {
                    stretch = sma(&closes, SMA_PERIOD).map(|s| (s - row.price) / atr14);
                    e3 = period_extremes(&bars, row.price, atr14, DAYS_3MO);
                    e1y = period_extremes(&bars, row.price, atr14, DAYS_1Y);
                    // A real trading year is ~250 days; only flag a span that
                    // is meaningfully shorter (young listing, thin history).
                    if e1y.as_ref().is_some_and(|e| e.days < 200) {
                        notes.push(format!(
                            "long-range extremes cover only {} trading days, not a full year",
                            e1y.as_ref().map(|e| e.days).unwrap_or(0)
                        ));
                    }
                }
            }
            rsi14 = rsi_cutler(&closes, RSI_PERIOD);
        }
    }

    let today = Utc::now()
        .with_timezone(&chrono_tz::America::New_York)
        .date_naive();
    let since = today.pred_opt().unwrap_or(today);
    let filings = match deps.filings.recent_filings(&ticker, since).await {
        Ok(f) => GateEvidence::Available(f),
        Err(e) => GateEvidence::Unavailable(e.to_string()),
    };
    let (headlines, company_names) = match deps.news.headlines(&ticker, HEADLINE_COUNT).await {
        Ok(fetch) => (
            GateEvidence::Available(fetch.headlines),
            fetch.company_names,
        ),
        Err(e) => (GateEvidence::Unavailable(e.to_string()), Vec::new()),
    };
    let (filing_status, mut evidence) = filing_gate(&filings, since);
    let (headline_status, headline_evidence) =
        headline_gate(&headlines, ticker.as_str(), &company_names);
    evidence.extend(headline_evidence);
    let catalyst = Some(CatalystCheck {
        filings: filing_status,
        headlines: headline_status,
        evidence,
    });

    let sentiment = crate::application::dip::sentiment_for(ticker.as_str(), deps.social).await;

    Candidate {
        ticker: ticker.as_str().to_string(),
        screens,
        change_pct: row.change_pct,
        price: row.price,
        rvol: rvol_of(row),
        stretch_atr: stretch,
        rsi14,
        extremes_3mo: e3,
        extremes_1y: e1y,
        catalyst,
        sentiment,
        notes,
    }
}

fn rvol_of(row: &MoverRow) -> Option<f64> {
    match (row.day_volume, row.avg_volume_3mo) {
        (Some(day), Some(avg)) if avg > 0 => Some(day as f64 / avg as f64),
        _ => None,
    }
}

pub async fn discover(
    req: &DiscoverRequest,
    deps: &DiscoverDeps<'_>,
    now: DateTime<Utc>,
) -> Result<DiscoverReport, DomainError> {
    if req.screens.is_empty() {
        return Err(fail("at least one screen is required"));
    }
    let count = req.count.clamp(1, 100);
    let now_ms = now.timestamp_millis();

    // Order-preserving dedupe: a repeated screen would double-fetch upstream
    // and then collide on the index map below.
    let mut screens: Vec<ScreenKind> = Vec::new();
    for kind in &req.screens {
        if !screens.contains(kind) {
            screens.push(*kind);
        }
    }

    let fetched: Vec<(ScreenKind, Result<Vec<MoverRow>, DomainError>)> = futures::stream::iter(
        screens
            .clone()
            .into_iter()
            .map(|kind| async move { (kind, deps.movers.screen(kind, count).await) }),
    )
    .buffer_unordered(CONCURRENCY)
    .collect()
    .await;
    // buffer_unordered scrambles completion order; restore request order.
    let mut by_kind: BTreeMap<usize, (ScreenKind, Result<Vec<MoverRow>, DomainError>)> =
        BTreeMap::new();
    for entry in fetched {
        let idx = screens.iter().position(|k| *k == entry.0).unwrap_or(0);
        by_kind.insert(idx, entry);
    }

    let mut errors = Vec::new();
    let mut summaries = Vec::new();
    // Per screen: floored rows in provider order, minus symbols already taken
    // by an earlier screen (first screen wins; later screens tag on).
    let mut groups: Vec<(ScreenKind, Vec<MoverRow>)> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let mut extra_tags: BTreeMap<String, Vec<ScreenKind>> = BTreeMap::new();

    for (_, (kind, result)) in by_kind {
        match result {
            Err(e) => {
                errors.push(format!("{}: screen failed: {e}", kind.label()));
                summaries.push(ScreenSummary {
                    screen: kind,
                    universe: 0,
                    floor_rejects: 0,
                    annotated: 0,
                });
            }
            Ok(rows) => {
                let universe = rows.len();
                let (passed, rejects) = apply_floor(&rows, &req.floor, now_ms);
                let mut fresh = Vec::new();
                for row in passed {
                    if seen.contains(&row.symbol) {
                        extra_tags.entry(row.symbol.clone()).or_default().push(kind);
                    } else {
                        seen.push(row.symbol.clone());
                        fresh.push(row);
                    }
                }
                summaries.push(ScreenSummary {
                    screen: kind,
                    universe,
                    floor_rejects: rejects.len(),
                    annotated: 0,
                });
                groups.push((kind, fresh));
            }
        }
    }

    let sizes: Vec<usize> = groups.iter().map(|(_, rows)| rows.len()).collect();
    let alloc = allocate_deep(&sizes, req.deep.clamp(1, 25));

    let mut picks: Vec<(ScreenKind, MoverRow)> = Vec::new();
    for ((kind, rows), take) in groups.into_iter().zip(alloc) {
        if let Some(s) = summaries.iter_mut().find(|s| s.screen == kind) {
            s.annotated = take;
        }
        picks.extend(rows.into_iter().take(take).map(|r| (kind, r)));
    }

    let mut indexed: Vec<(usize, Candidate)> =
        futures::stream::iter(picks.into_iter().enumerate().map(|(i, (kind, row))| {
            let mut screens = vec![kind];
            if let Some(extra) = extra_tags.get(&row.symbol) {
                screens.extend(extra.iter().copied());
            }
            async move { (i, annotate(&row, screens, deps).await) }
        }))
        .buffer_unordered(CONCURRENCY)
        .collect()
        .await;
    // Restore pick order after concurrent annotation.
    indexed.sort_by_key(|(i, _)| *i);
    let candidates: Vec<Candidate> = indexed.into_iter().map(|(_, c)| c).collect();

    let mut notes = Vec::new();
    if candidates
        .iter()
        .any(|c| c.screens.contains(&ScreenKind::DayLosers))
    {
        notes.push(
            "losers shown here carry no verdict — run dip_scan for the gated dip-setup grading"
                .into(),
        );
    }
    if deps.social.is_empty() {
        notes.push("no social sources configured — attention column unavailable".into());
    }

    Ok(DiscoverReport {
        generated_at: now,
        screens: summaries,
        candidates,
        errors,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::filings::mock_filings::MockFilingsSource;
    use crate::adapters::market::mock_news::MockNewsSource;
    use crate::domain::ports::news_source::NewsFetch;
    use crate::domain::values::bar::Bar;
    use crate::domain::values::mover::MoverRow;
    use async_trait::async_trait;
    use chrono::{NaiveDate, TimeZone};

    struct FixedBars;

    #[async_trait]
    impl BarSource for FixedBars {
        async fn bars(&self, _t: &Ticker) -> Result<Vec<Bar>, DomainError> {
            Ok((0..40)
                .map(|i| Bar {
                    date: NaiveDate::from_ymd_opt(2026, 6, 1)
                        .unwrap()
                        .checked_add_days(chrono::Days::new(i))
                        .unwrap(),
                    open: 55.0,
                    high: 60.0,
                    low: 50.0,
                    close: 55.0,
                })
                .collect())
        }
    }

    fn quiet_news() -> MockNewsSource {
        MockNewsSource(Ok(NewsFetch {
            headlines: vec![],
            company_names: vec![],
        }))
    }

    struct ScreenedMovers;

    #[async_trait]
    impl MoversSource for ScreenedMovers {
        fn name(&self) -> &str {
            "screened"
        }
        async fn screen(
            &self,
            kind: ScreenKind,
            _count: usize,
        ) -> Result<Vec<MoverRow>, DomainError> {
            let row = |symbol: &str, change: f64| MoverRow {
                symbol: symbol.into(),
                change_pct: change,
                price: 50.0,
                market_cap: Some(2_000_000_000),
                avg_volume_3mo: Some(5_000_000),
                day_volume: Some(10_000_000),
                exchange: "NYSE".into(),
                first_trade_ms: Some(0),
            };
            Ok(match kind {
                ScreenKind::DayGainers => vec![row("UPUP", 12.0), row("BOTH", 6.0)],
                ScreenKind::DayLosers => vec![row("DOWN", -9.0)],
                ScreenKind::MostActives => vec![row("BOTH", 6.0), row("BUSY", 1.0)],
            })
        }
    }

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 21, 21, 0, 0).unwrap()
    }

    #[tokio::test]
    async fn discover_merges_screens_dedupes_and_notes_losers() {
        let news = quiet_news();
        let filings = MockFilingsSource(Ok(vec![]));
        let deps = DiscoverDeps {
            movers: &ScreenedMovers,
            bars: &FixedBars,
            news: &news,
            filings: &filings,
            social: &[],
        };
        let report = discover(&DiscoverRequest::default(), &deps, now())
            .await
            .unwrap();

        assert_eq!(report.screens.len(), 3);
        let tickers: Vec<&str> = report
            .candidates
            .iter()
            .map(|c| c.ticker.as_str())
            .collect();
        assert!(tickers.contains(&"UPUP"));
        assert!(tickers.contains(&"DOWN"));
        // BOTH appears once, tagged with both screens.
        let both: Vec<&Candidate> = report
            .candidates
            .iter()
            .filter(|c| c.ticker == "BOTH")
            .collect();
        assert_eq!(both.len(), 1);
        assert!(both[0].screens.contains(&ScreenKind::DayGainers));
        assert!(both[0].screens.contains(&ScreenKind::MostActives));
        assert!((both[0].rvol.unwrap() - 2.0).abs() < 1e-12);
        assert!(report.notes.iter().any(|n| n.contains("dip_scan")));
        assert!(report.notes.iter().any(|n| n.contains("no social sources")));
    }

    #[tokio::test]
    async fn duplicate_screens_collapse_to_one_fetch() {
        let news = quiet_news();
        let filings = MockFilingsSource(Ok(vec![]));
        let deps = DiscoverDeps {
            movers: &ScreenedMovers,
            bars: &FixedBars,
            news: &news,
            filings: &filings,
            social: &[],
        };
        let req = DiscoverRequest {
            screens: vec![ScreenKind::DayLosers, ScreenKind::DayLosers],
            ..DiscoverRequest::default()
        };
        let report = discover(&req, &deps, now()).await.unwrap();
        assert_eq!(report.screens.len(), 1);
        assert_eq!(report.candidates.len(), 1);
        assert_eq!(report.candidates[0].ticker, "DOWN");
    }

    #[tokio::test]
    async fn screen_failure_is_an_error_not_a_sunk_run() {
        struct HalfBroken;
        #[async_trait]
        impl MoversSource for HalfBroken {
            fn name(&self) -> &str {
                "half"
            }
            async fn screen(
                &self,
                kind: ScreenKind,
                count: usize,
            ) -> Result<Vec<MoverRow>, DomainError> {
                match kind {
                    ScreenKind::DayGainers => Err(DomainError::SourceFailure {
                        name: "x".into(),
                        message: "boom".into(),
                    }),
                    _ => ScreenedMovers.screen(kind, count).await,
                }
            }
        }
        let news = quiet_news();
        let filings = MockFilingsSource(Ok(vec![]));
        let deps = DiscoverDeps {
            movers: &HalfBroken,
            bars: &FixedBars,
            news: &news,
            filings: &filings,
            social: &[],
        };
        let report = discover(&DiscoverRequest::default(), &deps, now())
            .await
            .unwrap();
        assert!(report.errors.iter().any(|e| e.contains("gainers")));
        assert!(!report.candidates.is_empty());
    }
}
