//! dip_scan / dip_check orchestration: fetch the losers universe, gate
//! evidence (bars, filings, headlines, social), run the pure domain signal,
//! and overlay risk + margin frames. IO and the clock live here; the verdict
//! logic lives in `domain::dip`.

use std::path::PathBuf;

use chrono::{DateTime, Datelike, Timelike, Utc, Weekday};
use futures::StreamExt;
use serde::Serialize;

use crate::application::request::AnalysisRequest;
use crate::domain::dip::{
    apply_floor, dip_signal, DipInputs, DipSignal, DropBand, FloorReject, GateEvidence, GateStatus,
    QualityFloor, ScoreWeights, SentimentSummary, Session, Verdict,
};
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::margin::{margin_frame, MarginFrame, MarginInputs};
use crate::domain::ports::bar_source::BarSource;
use crate::domain::ports::filings_source::FilingsSource;
use crate::domain::ports::market_data_source::MarketDataSource;
use crate::domain::ports::movers_source::MoversSource;
use crate::domain::ports::news_source::NewsSource;
use crate::domain::ports::social_data_source::SocialDataSource;
use crate::domain::risk::{frame, Direction, RiskFrame};
use crate::domain::values::bar::Bar;
use crate::domain::values::mover::MoverRow;
use crate::domain::values::source_kind::SourceKind;

pub const DEFAULT_SCORE_MIN: f64 = 65.0;
pub const DEFAULT_DEEP_N: usize = 10;
pub const MAX_DEEP_N: usize = 25;
/// Concurrent per-candidate fan-out — bounded for SEC's ≤10 req/s policy.
const CONCURRENCY: usize = 5;
/// Equity ETF proxy for the S&P 500 (index symbols don't parse as tickers).
pub const SPX_PROXY: &str = "SPY";
const HEADLINE_COUNT: usize = 20;
const SOCIAL_LIMIT: usize = 50;

fn fail(message: impl Into<String>) -> DomainError {
    DomainError::SourceFailure {
        name: "dip".into(),
        message: message.into(),
    }
}

/// Everything dip_check needs besides the universe feed.
pub struct DipDeps<'a> {
    pub bars: &'a dyn BarSource,
    pub news: &'a dyn NewsSource,
    pub filings: &'a dyn FilingsSource,
    pub social: &'a [Box<dyn SocialDataSource>],
    /// Fills day/avg volume in single-ticker mode (scan mode reads the
    /// screener row instead).
    pub market: Option<&'a dyn MarketDataSource>,
}

#[derive(Debug, Clone)]
pub struct DipScanRequest {
    pub count: usize,
    pub deep_n: usize,
    pub band: DropBand,
    pub floor: QualityFloor,
    pub weights: ScoreWeights,
    pub score_min: f64,
    pub margin: Option<MarginInputs>,
    /// None = no journal write.
    pub journal_path: Option<PathBuf>,
}

impl Default for DipScanRequest {
    fn default() -> Self {
        Self {
            count: 100,
            deep_n: DEFAULT_DEEP_N,
            band: DropBand::default(),
            floor: QualityFloor::default(),
            weights: ScoreWeights::default(),
            score_min: DEFAULT_SCORE_MIN,
            margin: None,
            journal_path: None,
        }
    }
}

/// `~/.openintel/dip_journal.jsonl`, or None when no home dir resolves.
pub fn default_journal_path() -> Option<PathBuf> {
    std::env::home_dir().map(|h| h.join(".openintel").join("dip_journal.jsonl"))
}

#[derive(Debug, Serialize)]
pub struct DipCandidate {
    pub ticker: String,
    pub change_pct: f64,
    /// Drop-day close (intraday: last traded price of the partial bar).
    pub price: f64,
    pub signal: DipSignal,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub risk: Option<RiskFrame>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin: Option<MarginFrame>,
}

#[derive(Debug, Serialize)]
pub struct EntryError {
    pub ticker: String,
    pub error: String,
}

#[derive(Debug, Serialize)]
pub struct DipScanReport {
    pub generated_at: DateTime<Utc>,
    pub session: Session,
    pub spx_proxy: &'static str,
    pub spx_change_pct: Option<f64>,
    pub universe_size: usize,
    pub band_excluded: usize,
    pub floor_rejects: Vec<FloorReject>,
    pub candidates: Vec<DipCandidate>,
    pub errors: Vec<EntryError>,
    pub notes: Vec<String>,
}

/// Trading-session state in New York. Weekends and anything outside
/// 09:30–16:05 ET count as post-close (the last completed session is final).
pub fn session_for(now: DateTime<Utc>) -> Session {
    let ny = now.with_timezone(&chrono_tz::America::New_York);
    if matches!(ny.weekday(), Weekday::Sat | Weekday::Sun) {
        return Session::PostClose;
    }
    let minutes = ny.hour() * 60 + ny.minute();
    if (9 * 60 + 30..16 * 60 + 5).contains(&minutes) {
        Session::Intraday
    } else {
        Session::PostClose
    }
}

/// Split history into (prior bars, drop-day bar). The drop day is the last
/// bar's session — final post-close, partial intraday.
fn split_history(bars: &[Bar]) -> Result<(&[Bar], Bar), DomainError> {
    match bars.split_last() {
        Some((last, prior)) if !prior.is_empty() => Ok((prior, *last)),
        _ => Err(fail("no price history")),
    }
}

fn change_pct_from(prior: &[Bar], last: &Bar) -> Result<f64, DomainError> {
    let prev_close = prior
        .last()
        .map(|b| b.close)
        .ok_or_else(|| fail("no previous close"))?;
    if prev_close <= 0.0 {
        return Err(fail("previous close is not positive"));
    }
    Ok((last.close - prev_close) / prev_close * 100.0)
}

/// The proxy index's drop-day change, from its own bar history.
async fn spx_change(bars_src: &dyn BarSource) -> Result<f64, DomainError> {
    let ticker = Ticker::parse(SPX_PROXY)?;
    let bars = bars_src.bars(&ticker).await?;
    let (prior, last) = split_history(&bars)?;
    change_pct_from(prior, &last)
}

/// Social-only sentiment for the divergence component. Any failure (including
/// "no posts") degrades to None — the domain scores divergence 0 with a note.
async fn sentiment_for(
    ticker: &str,
    social: &[Box<dyn SocialDataSource>],
) -> Option<SentimentSummary> {
    if social.is_empty() {
        return None;
    }
    let req = AnalysisRequest {
        ticker: ticker.to_string(),
        enabled_sources: SourceKind::ALL.to_vec(),
        market_enabled: false,
        limit: SOCIAL_LIMIT,
        engine: crate::domain::engine::config::EngineConfig::default(),
    };
    let report = crate::application::analyze(&req, social, None).await.ok()?;
    Some(SentimentSummary {
        net_sentiment: report.social.net_sentiment.value(),
        mentions: report.social.total_mentions,
    })
}

struct CheckContext {
    change_pct: Option<f64>,
    day_volume: Option<u64>,
    avg_volume_3mo: Option<u64>,
    floor: GateStatus,
}

/// Evaluate one ticker end-to-end and return the signal plus the bar history
/// (so callers can size a risk frame without refetching).
async fn check(
    ticker_raw: &str,
    ctx: CheckContext,
    spx_change_pct: Option<f64>,
    req: &DipScanRequest,
    deps: &DipDeps<'_>,
    now: DateTime<Utc>,
) -> Result<(DipSignal, Vec<Bar>), DomainError> {
    let ticker = Ticker::parse(ticker_raw)?;
    let session = session_for(now);

    let bars = deps.bars.bars(&ticker).await?;
    let (prior, drop_bar) = split_history(&bars)?;
    let drop_date = drop_bar.date;
    let change_pct = match ctx.change_pct {
        Some(c) => c,
        None => change_pct_from(prior, &drop_bar)?,
    };

    let filings = {
        let since = drop_date.pred_opt().unwrap_or(drop_date);
        match deps.filings.recent_filings(&ticker, since).await {
            Ok(filings) => GateEvidence::Available(filings),
            Err(e) => GateEvidence::Unavailable(e.to_string()),
        }
    };

    let headlines = match deps.news.headlines(&ticker, HEADLINE_COUNT).await {
        Ok(all) => GateEvidence::Available(
            all.into_iter()
                .filter(|h| {
                    h.published_at
                        .with_timezone(&chrono_tz::America::New_York)
                        .date_naive()
                        == drop_date
                })
                .collect(),
        ),
        Err(e) => GateEvidence::Unavailable(e.to_string()),
    };

    let sentiment = sentiment_for(ticker.as_str(), deps.social).await;

    let signal = dip_signal(&DipInputs {
        ticker: ticker.as_str(),
        prior_bars: prior,
        drop_day_bar: Some(drop_bar),
        drop_date,
        last_price: drop_bar.close,
        change_pct,
        day_volume: ctx.day_volume,
        avg_volume_3mo: ctx.avg_volume_3mo,
        spx_change_pct,
        filings,
        headlines,
        sentiment,
        session,
        floor: ctx.floor,
        band: req.band,
        weights: &req.weights,
        score_min: req.score_min,
    })?;
    Ok((signal, bars))
}

/// Single-ticker mode — the "considering a trade" entry point. The quality
/// floor is Unknown here (no screener row to evaluate), so the verdict caps
/// at `watch`; volumes come from a market snapshot when one is wired.
pub async fn dip_check(
    ticker_raw: &str,
    req: &DipScanRequest,
    deps: &DipDeps<'_>,
    now: DateTime<Utc>,
) -> Result<DipSignal, DomainError> {
    let (day_volume, avg_volume_3mo) = match deps.market {
        Some(market) => match market.snapshot(&Ticker::parse(ticker_raw)?).await {
            Ok(snap) => (Some(snap.volume), Some(snap.avg_volume)),
            Err(_) => (None, None),
        },
        None => (None, None),
    };
    let spx = spx_change(deps.bars).await.ok();
    let ctx = CheckContext {
        change_pct: None,
        day_volume,
        avg_volume_3mo,
        floor: GateStatus::Unknown(
            "quality floor not evaluated — ticker not from the screener universe".into(),
        ),
    };
    check(ticker_raw, ctx, spx, req, deps, now)
        .await
        .map(|(signal, _)| signal)
}

fn verdict_rank(v: Verdict) -> u8 {
    match v {
        Verdict::HighConfidence => 2,
        Verdict::Watch => 1,
        Verdict::NoSetup => 0,
    }
}

/// Universe mode: losers list → quality floor → drop band → bounded
/// concurrent dip_check → risk/margin overlay → rank → journal.
pub async fn dip_scan(
    req: &DipScanRequest,
    movers: &dyn MoversSource,
    deps: &DipDeps<'_>,
    now: DateTime<Utc>,
) -> Result<DipScanReport, DomainError> {
    let rows = movers.day_losers(req.count).await?;
    let universe_size = rows.len();
    let mut notes: Vec<String> = Vec::new();

    let (survivors, floor_rejects) = apply_floor(&rows, &req.floor, now.timestamp_millis());
    let (eligible, band_excluded): (Vec<MoverRow>, Vec<MoverRow>) = survivors
        .into_iter()
        .partition(|r| r.change_pct >= req.band.min_pct && r.change_pct <= req.band.max_pct);
    let band_excluded = band_excluded.len();

    let deep_n = req.deep_n.clamp(1, MAX_DEEP_N);
    if eligible.len() > deep_n {
        notes.push(format!(
            "{} band-eligible candidates — deep-analyzing the worst {deep_n} (raise deep_n to widen)",
            eligible.len()
        ));
    }
    let deep: Vec<MoverRow> = eligible.into_iter().take(deep_n).collect();

    let spx_change_pct = match spx_change(deps.bars).await {
        Ok(c) => Some(c),
        Err(e) => {
            notes.push(format!("index proxy {SPX_PROXY} unavailable: {e}"));
            None
        }
    };

    type CheckOutcome = Result<(DipSignal, Vec<Bar>), DomainError>;
    let results: Vec<(String, f64, CheckOutcome)> =
        futures::stream::iter(deep.into_iter().map(|row| async move {
            let ctx = CheckContext {
                change_pct: Some(row.change_pct),
                day_volume: row.day_volume,
                avg_volume_3mo: row.avg_volume_3mo,
                floor: GateStatus::Pass,
            };
            let outcome = check(&row.symbol, ctx, spx_change_pct, req, deps, now).await;
            (row.symbol, row.change_pct, outcome)
        }))
        .buffer_unordered(CONCURRENCY)
        .collect()
        .await;

    let mut candidates: Vec<DipCandidate> = Vec::new();
    let mut errors: Vec<EntryError> = Vec::new();
    for (ticker, change_pct, outcome) in results {
        match outcome {
            Ok((signal, bars)) => {
                let (risk, margin) = match &req.margin {
                    Some(inputs) => risk_and_margin(&ticker, &bars, inputs, now, &mut notes),
                    None => (None, None),
                };
                let price = bars.last().map(|b| b.close).unwrap_or_default();
                candidates.push(DipCandidate {
                    ticker,
                    change_pct,
                    price,
                    signal,
                    risk,
                    margin,
                });
            }
            Err(e) => errors.push(EntryError {
                ticker,
                error: e.to_string(),
            }),
        }
    }

    candidates.sort_by(|a, b| {
        verdict_rank(b.signal.verdict)
            .cmp(&verdict_rank(a.signal.verdict))
            .then_with(|| {
                b.signal
                    .score
                    .partial_cmp(&a.signal.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    let mut report = DipScanReport {
        generated_at: now,
        session: session_for(now),
        spx_proxy: SPX_PROXY,
        spx_change_pct,
        universe_size,
        band_excluded,
        floor_rejects,
        candidates,
        errors,
        notes,
    };
    if let Some(path) = &req.journal_path {
        if let Err(e) = append_journal(path, &report) {
            report.notes.push(format!("journal write failed: {e}"));
        }
    }
    Ok(report)
}

/// Size a long risk frame off the fetched bars, then overlay margin.
/// Failures degrade to None with a note — sizing must never sink the scan.
fn risk_and_margin(
    ticker: &str,
    bars: &[Bar],
    inputs: &MarginInputs,
    now: DateTime<Utc>,
    notes: &mut Vec<String>,
) -> (Option<RiskFrame>, Option<MarginFrame>) {
    let risk_pct = inputs.risk_pct.clamp(0.001, 0.05);
    let budget = inputs.equity_usd * risk_pct;
    let entry = match bars.last() {
        Some(b) => b.close,
        None => return (None, None),
    };
    let risk = match frame(ticker, bars, Direction::Long, entry, budget, 2.0, now) {
        Ok(r) => r,
        Err(e) => {
            notes.push(format!("{ticker}: risk frame unavailable: {e}"));
            return (None, None);
        }
    };
    match margin_frame(&risk, inputs) {
        Ok(m) => (Some(risk), Some(m)),
        Err(e) => {
            notes.push(format!("{ticker}: margin frame unavailable: {e}"));
            (Some(risk), None)
        }
    }
}

/// One compact JSONL line per scan — the record that lets the v0 weights be
/// graded against forward returns later.
#[derive(Serialize)]
struct JournalLine<'a> {
    generated_at: DateTime<Utc>,
    session: Session,
    spx_change_pct: Option<f64>,
    candidates: Vec<JournalCandidate<'a>>,
}

#[derive(Serialize)]
struct JournalCandidate<'a> {
    ticker: &'a str,
    change_pct: f64,
    price: f64,
    score: f64,
    verdict: Verdict,
}

fn append_journal(path: &PathBuf, report: &DipScanReport) -> Result<(), String> {
    use std::io::Write as _;
    let line = JournalLine {
        generated_at: report.generated_at,
        session: report.session,
        spx_change_pct: report.spx_change_pct,
        candidates: report
            .candidates
            .iter()
            .map(|c| JournalCandidate {
                ticker: &c.ticker,
                change_pct: c.change_pct,
                price: c.price,
                score: c.signal.score,
                verdict: c.signal.verdict,
            })
            .collect(),
    };
    let json = serde_json::to_string(&line).map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    writeln!(file, "{json}").map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::filings::mock_filings::MockFilingsSource;
    use crate::adapters::market::mock_movers::MockMoversSource;
    use crate::adapters::market::mock_news::MockNewsSource;
    use crate::adapters::sources::test_fixtures::fixture_social;
    use crate::domain::values::filing::Filing;
    use async_trait::async_trait;
    use chrono::{NaiveDate, TimeZone};
    use std::collections::HashMap;

    /// Friday 2026-08-14 17:30 New York = 21:30 UTC (EDT) — post-close.
    fn post_close_now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 14, 21, 30, 0).unwrap()
    }

    fn d(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, day).unwrap()
    }

    /// 30 prior bars gently declining + a drop-day bar that closes strong.
    fn dip_bars(drop_close: f64) -> Vec<Bar> {
        let mut v: Vec<Bar> = (1..=30)
            .map(|i| {
                let c = 110.0 - i as f64 * 0.5;
                Bar {
                    date: d(i),
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

    /// Flat index proxy: tiny change so candidate moves are idiosyncratic.
    fn spy_bars() -> Vec<Bar> {
        (1..=31)
            .map(|i| Bar {
                date: d(i.min(30)).succ_opt().unwrap(),
                open: 500.0,
                high: 501.0,
                low: 499.0,
                close: if i == 31 { 499.0 } else { 500.0 },
            })
            .collect()
    }

    struct MapBars(HashMap<String, Vec<Bar>>);

    #[async_trait]
    impl BarSource for MapBars {
        async fn bars(&self, t: &Ticker) -> Result<Vec<Bar>, DomainError> {
            self.0
                .get(t.as_str())
                .cloned()
                .ok_or_else(|| DomainError::SourceFailure {
                    name: "map-bars".into(),
                    message: format!("no bars for {}", t.as_str()),
                })
        }
    }

    fn row(symbol: &str, change_pct: f64) -> MoverRow {
        MoverRow {
            symbol: symbol.into(),
            change_pct,
            price: 87.0,
            market_cap: Some(2_000_000_000),
            avg_volume_3mo: Some(5_000_000),
            day_volume: Some(4_000_000),
            exchange: "NYSE".into(),
            first_trade_ms: Some(0),
        }
    }

    fn bars_map() -> MapBars {
        let mut m = HashMap::new();
        m.insert("GOOD".to_string(), dip_bars(87.4));
        m.insert("FILED".to_string(), dip_bars(87.4));
        m.insert(SPX_PROXY.to_string(), spy_bars());
        // NOBAR deliberately absent
        MapBars(m)
    }

    #[test]
    fn session_boundaries_in_new_york() {
        // Sat noon UTC
        let sat = Utc.with_ymd_and_hms(2026, 8, 15, 12, 0, 0).unwrap();
        assert_eq!(session_for(sat), Session::PostClose);
        // Friday 12:00 NY = 16:00 UTC (EDT)
        let midday = Utc.with_ymd_and_hms(2026, 8, 14, 16, 0, 0).unwrap();
        assert_eq!(session_for(midday), Session::Intraday);
        // Friday 16:10 NY
        let after = Utc.with_ymd_and_hms(2026, 8, 14, 20, 10, 0).unwrap();
        assert_eq!(session_for(after), Session::PostClose);
        // Friday 08:00 NY (pre-open: last session is final)
        let pre = Utc.with_ymd_and_hms(2026, 8, 14, 12, 0, 0).unwrap();
        assert_eq!(session_for(pre), Session::PostClose);
    }

    fn clean_deps<'a>(
        bars: &'a MapBars,
        news: &'a MockNewsSource,
        filings: &'a MockFilingsSource,
        social: &'a [Box<dyn SocialDataSource>],
    ) -> DipDeps<'a> {
        DipDeps {
            bars,
            news,
            filings,
            social,
            market: None,
        }
    }

    #[tokio::test]
    async fn scan_ranks_verdicts_and_collects_errors() {
        let bars = bars_map();
        let news = MockNewsSource(Ok(vec![]));
        let filings = MockFilingsSource(Ok(vec![]));
        let social = fixture_social();
        let movers = MockMoversSource(vec![
            row("CRASH", -30.0), // outside band -> excluded pre-analysis
            row("GOOD", -8.0),   // clean candidate
            row("NOBAR", -7.0),  // bar fetch fails -> entry error
            MoverRow {
                price: 2.0,
                ..row("CHEAP", -9.0) // floor reject
            },
        ]);
        let req = DipScanRequest::default();
        let deps = clean_deps(&bars, &news, &filings, &social);
        let report = dip_scan(&req, &movers, &deps, post_close_now())
            .await
            .unwrap();

        assert_eq!(report.universe_size, 4);
        assert_eq!(report.band_excluded, 1);
        assert_eq!(report.floor_rejects.len(), 1);
        assert_eq!(report.floor_rejects[0].symbol, "CHEAP");
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.errors[0].ticker, "NOBAR");
        assert_eq!(report.candidates.len(), 1);
        let good = &report.candidates[0];
        assert_eq!(good.ticker, "GOOD");
        assert_eq!(good.signal.verdict, Verdict::HighConfidence);
        assert!(good.risk.is_none() && good.margin.is_none()); // no equity given
        assert_eq!(report.session, Session::PostClose);
        assert!(report.spx_change_pct.unwrap().abs() < 1.0);
    }

    #[tokio::test]
    async fn scan_fails_closed_when_edgar_down_and_kills_on_filing() {
        let bars = bars_map();
        let news = MockNewsSource(Ok(vec![]));
        let social = fixture_social();
        let movers = MockMoversSource(vec![row("GOOD", -8.0)]);
        let req = DipScanRequest::default();

        // EDGAR down -> watch, never high_confidence
        let filings = MockFilingsSource(Err("EDGAR unreachable".into()));
        let deps = clean_deps(&bars, &news, &filings, &social);
        let report = dip_scan(&req, &movers, &deps, post_close_now())
            .await
            .unwrap();
        assert_eq!(report.candidates[0].signal.verdict, Verdict::Watch);

        // 8-K on the drop day -> no_setup with evidence
        let filings = MockFilingsSource(Ok(vec![Filing {
            form: "8-K".into(),
            filed_on: NaiveDate::from_ymd_opt(2026, 8, 14).unwrap(),
        }]));
        let deps = clean_deps(&bars, &news, &filings, &social);
        let report = dip_scan(&req, &movers, &deps, post_close_now())
            .await
            .unwrap();
        assert_eq!(report.candidates[0].signal.verdict, Verdict::NoSetup);
        assert!(!report.candidates[0].signal.catalyst_evidence.is_empty());
    }

    #[tokio::test]
    async fn margin_section_present_only_with_equity_and_journal_written() {
        let bars = bars_map();
        let news = MockNewsSource(Ok(vec![]));
        let filings = MockFilingsSource(Ok(vec![]));
        let social = fixture_social();
        let movers = MockMoversSource(vec![row("GOOD", -8.0)]);

        let journal = std::env::temp_dir().join(format!(
            "openintel-dip-test-{}/journal.jsonl",
            std::process::id()
        ));
        let req = DipScanRequest {
            margin: Some(MarginInputs {
                equity_usd: 10_000.0,
                ..MarginInputs::default()
            }),
            journal_path: Some(journal.clone()),
            ..DipScanRequest::default()
        };
        let deps = clean_deps(&bars, &news, &filings, &social);
        let report = dip_scan(&req, &movers, &deps, post_close_now())
            .await
            .unwrap();

        let c = &report.candidates[0];
        let risk = c.risk.as_ref().unwrap();
        assert!((risk.budget_usd - 100.0).abs() < 1e-9); // 1% of 10k
        let margin = c.margin.as_ref().unwrap();
        assert_eq!(margin.leverage, 2.0);
        assert!(margin.shares <= risk.shares);

        let line = std::fs::read_to_string(&journal).unwrap();
        assert!(line.contains("\"ticker\":\"GOOD\""));
        assert!(line.contains("\"verdict\":\"high_confidence\""));
        assert!(line.trim().lines().count() == 1);
        let _ = std::fs::remove_dir_all(journal.parent().unwrap());
    }

    #[tokio::test]
    async fn single_ticker_mode_caps_at_watch() {
        let bars = bars_map();
        let news = MockNewsSource(Ok(vec![]));
        let filings = MockFilingsSource(Ok(vec![]));
        let social = fixture_social();
        let req = DipScanRequest::default();
        let deps = clean_deps(&bars, &news, &filings, &social);
        let signal = dip_check("GOOD", &req, &deps, post_close_now())
            .await
            .unwrap();
        // clean everything, but the floor is unverifiable -> watch, not high
        assert_eq!(signal.verdict, Verdict::Watch);
        assert!(matches!(signal.gates.quality_floor, GateStatus::Unknown(_)));
        // change computed from bars: 95 -> 87.4 is ~-8%
        assert!((signal.metrics.excess_vs_spx_pct.unwrap() + 7.8).abs() < 0.5);
    }
}
