//! IO edge for `watch`: one poll cycle over the watched names, the events it
//! produced, and the append-only events file a session reads back. The loop
//! itself lives in the CLI; this is what one tick does. Keyless polling, so
//! every event carries the poll time as its latency ceiling.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use chrono_tz::America::New_York;
use futures::StreamExt;

use crate::application::chatter::{chatter, ChatterRequest};
use crate::domain::clock::market_clock;
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::ports::bar_source::BarSource;
use crate::domain::ports::filings_source::FilingsSource;
use crate::domain::ports::listening_feed::ListeningFeed;
use crate::domain::ports::market_data_source::MarketDataSource;
use crate::domain::ports::news_source::NewsSource;
use crate::domain::risk::{atr, ATR_PERIOD};
use crate::domain::values::asset_class::AssetClass;
use crate::domain::values::event::Event;
use crate::domain::watch::{
    chatter_event, filing_events, headline_events, market_state_event, move_events, Move,
    WatchState,
};

const CONCURRENCY: usize = 4;
const HEADLINE_COUNT: usize = 20;

pub const FRAMING: &str =
    "watch emits dated facts as they are seen — it never ranks, never predicts, \
     and never places or suggests an order.";

pub fn default_events_path() -> Option<PathBuf> {
    std::env::home_dir().map(|h| h.join(".openintel").join("events.jsonl"))
}

fn fail(message: impl Into<String>) -> DomainError {
    DomainError::SourceFailure {
        name: "watch".into(),
        message: message.into(),
    }
}

pub struct WatchDeps<'a> {
    pub bars: &'a dyn BarSource,
    pub market: &'a dyn MarketDataSource,
    pub news: &'a dyn NewsSource,
    pub filings: &'a dyn FilingsSource,
}

pub struct WatchRequest {
    pub tickers: Vec<Ticker>,
    /// Whole ATR multiples from the prior close that each earn one event.
    pub move_threshold_atr: f64,
}

/// ATR per ticker, fetched from daily bars once per process: the scale of a
/// move does not change within a session.
#[derive(Default)]
pub struct WatchCache {
    atr: BTreeMap<String, f64>,
}

#[derive(Debug)]
pub struct PollOutcome {
    pub polled_at: DateTime<Utc>,
    pub events: Vec<Event>,
    pub errors: Vec<String>,
}

async fn atr_for(
    ticker: &Ticker,
    cache: &mut WatchCache,
    bars: &dyn BarSource,
) -> Result<f64, DomainError> {
    if let Some(a) = cache.atr.get(ticker.as_str()) {
        return Ok(*a);
    }
    let history = bars.bars(ticker).await?;
    let a = atr(&history, ATR_PERIOD)
        .filter(|a| a.is_finite() && *a > 0.0)
        .ok_or_else(|| fail(format!("{}: not enough history for ATR", ticker.as_str())))?;
    cache.atr.insert(ticker.as_str().to_string(), a);
    Ok(a)
}

struct Observation {
    ticker: Ticker,
    quote: Result<(f64, f64), DomainError>,
    filings: Option<Result<Vec<crate::domain::values::filing::Filing>, DomainError>>,
    news: Option<Result<crate::domain::ports::news_source::NewsFetch, DomainError>>,
}

async fn observe(ticker: Ticker, since: chrono::NaiveDate, deps: &WatchDeps<'_>) -> Observation {
    let quote = deps
        .market
        .snapshot(&ticker)
        .await
        .map(|s| (s.last_price, s.previous_close));
    let equity = ticker.class() == AssetClass::Equity;
    let filings = if equity {
        Some(deps.filings.recent_filings(&ticker, since).await)
    } else {
        None
    };
    let news = if equity {
        Some(deps.news.headlines(&ticker, HEADLINE_COUNT).await)
    } else {
        None
    };
    Observation {
        ticker,
        quote,
        filings,
        news,
    }
}

/// One tick: the market clock, then every watched name concurrently.
pub async fn poll(
    req: &WatchRequest,
    deps: &WatchDeps<'_>,
    state: &mut WatchState,
    cache: &mut WatchCache,
    now: DateTime<Utc>,
) -> PollOutcome {
    let mut events = Vec::new();
    let mut errors = Vec::new();

    let clock = market_clock(now);
    events.extend(market_state_event(state, &clock, now));
    let today = now.with_timezone(&New_York).date_naive();

    let observations: Vec<Observation> = futures::stream::iter(
        req.tickers
            .iter()
            .cloned()
            .map(|t| async move { observe(t, today, deps).await }),
    )
    .buffer_unordered(CONCURRENCY)
    .collect()
    .await;

    for obs in observations {
        let symbol = obs.ticker.as_str().to_string();
        match obs.quote {
            Ok((last, prior_close)) => match atr_for(&obs.ticker, cache, deps.bars).await {
                Ok(a) => events.extend(move_events(
                    state,
                    &symbol,
                    Move {
                        last,
                        prior_close,
                        atr: a,
                        threshold_atr: req.move_threshold_atr,
                    },
                    today,
                    now,
                )),
                Err(e) => errors.push(e.to_string()),
            },
            Err(e) => errors.push(format!("{symbol}: quote unavailable: {e}")),
        }
        match obs.filings {
            Some(Ok(filings)) => events.extend(filing_events(state, &symbol, &filings, now)),
            Some(Err(e)) => errors.push(format!("{symbol}: filings unavailable: {e}")),
            None => {}
        }
        match obs.news {
            Some(Ok(fetch)) => events.extend(headline_events(
                state,
                &symbol,
                &fetch.headlines,
                &fetch.company_names,
                now,
            )),
            Some(Err(e)) => errors.push(format!("{symbol}: headlines unavailable: {e}")),
            None => {}
        }
    }

    PollOutcome {
        polled_at: now,
        events,
        errors,
    }
}

/// The chatter leg: a read-only velocity pass over the free listening feeds.
/// Never writes the baseline, so a loop can't fake a day's worth of lines.
pub async fn poll_chatter(
    req: &ChatterRequest,
    feeds: &[&dyn ListeningFeed],
    market: Option<&dyn MarketDataSource>,
    state: &mut WatchState,
    now: DateTime<Utc>,
) -> PollOutcome {
    let mut events = Vec::new();
    let mut errors = Vec::new();
    let today = now.with_timezone(&New_York).date_naive();
    let read_only = ChatterRequest {
        write_baseline: false,
        ..req.clone()
    };
    match chatter(&read_only, feeds, market, now).await {
        Ok(report) => {
            errors.extend(report.errors);
            for platform in &report.platforms {
                for t in &platform.tickers {
                    events.extend(chatter_event(
                        state,
                        &platform.platform,
                        &t.ticker,
                        t.velocity.elevated,
                        t.velocity.ratio,
                        today,
                        now,
                    ));
                }
            }
        }
        Err(e) => errors.push(format!("chatter unavailable: {e}")),
    }
    PollOutcome {
        polled_at: now,
        events,
        errors,
    }
}

pub fn append_events(path: &Path, events: &[Event]) -> Result<(), DomainError> {
    if events.is_empty() {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| fail(format!("create {}: {e}", dir.display())))?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| fail(format!("open {}: {e}", path.display())))?;
    for event in events {
        let line = serde_json::to_string(event).map_err(|e| fail(e.to_string()))?;
        writeln!(file, "{line}").map_err(|e| fail(format!("write {}: {e}", path.display())))?;
    }
    Ok(())
}

/// Events polled at or after `since`, newest first, at most `limit`. A
/// missing file is an empty history; an unreadable one is an error.
pub fn events_since(
    path: &Path,
    since: DateTime<Utc>,
    limit: usize,
) -> Result<(Vec<Event>, usize), DomainError> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((Vec::new(), 0)),
        Err(e) => return Err(fail(format!("read {}: {e}", path.display()))),
    };
    let mut malformed = 0;
    let mut events: Vec<Event> = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| match serde_json::from_str::<Event>(l) {
            Ok(e) => Some(e),
            Err(_) => {
                malformed += 1;
                None
            }
        })
        .filter(|e| e.polled_at >= since)
        .collect();
    events.sort_by_key(|e| std::cmp::Reverse(e.polled_at));
    events.truncate(limit);
    Ok((events, malformed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::filings::mock_filings::MockFilingsSource;
    use crate::adapters::market::mock_market::MockMarketSource;
    use crate::adapters::market::mock_news::MockNewsSource;
    use crate::domain::ports::news_source::NewsFetch;
    use crate::domain::values::bar::Bar;
    use crate::domain::values::event::EventKind;
    use crate::domain::values::filing::Filing;
    use async_trait::async_trait;
    use chrono::{NaiveDate, TimeZone};

    struct FixedBars(Vec<Bar>);

    #[async_trait]
    impl BarSource for FixedBars {
        async fn bars(&self, _t: &Ticker) -> Result<Vec<Bar>, DomainError> {
            Ok(self.0.clone())
        }
    }

    fn history() -> Vec<Bar> {
        (0..16)
            .map(|_| Bar {
                date: NaiveDate::from_ymd_opt(2026, 9, 4).unwrap(),
                open: 190.0,
                high: 194.0,
                low: 186.0,
                close: 190.0,
            })
            .collect()
    }

    fn tuesday_open() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 8, 14, 0, 0).unwrap()
    }

    #[tokio::test]
    async fn a_poll_reports_each_fact_once_and_keeps_errors() {
        let bars = FixedBars(history());
        let filings = MockFilingsSource(Ok(vec![Filing {
            form: "8-K".into(),
            filed_on: NaiveDate::from_ymd_opt(2026, 9, 8).unwrap(),
            accession: None,
        }]));
        let news = MockNewsSource(Err("news down".into()));
        let deps = WatchDeps {
            bars: &bars,
            market: &MockMarketSource,
            news: &news,
            filings: &filings,
        };
        let req = WatchRequest {
            tickers: vec![Ticker::parse("AAPL").unwrap()],
            move_threshold_atr: 1.0,
        };
        let mut state = WatchState::default();
        let mut cache = WatchCache::default();

        let first = poll(&req, &deps, &mut state, &mut cache, tuesday_open()).await;
        // mock snapshot: 192.50 vs 185.00 on ATR 8 = 0.94 ATR, under one step
        assert!(first.events.iter().any(|e| e.kind == EventKind::Filing));
        assert!(!first.events.iter().any(|e| e.kind == EventKind::PriceMove));
        assert!(first
            .errors
            .iter()
            .any(|e| e.contains("headlines unavailable")));

        let second = poll(&req, &deps, &mut state, &mut cache, tuesday_open()).await;
        assert!(second.events.is_empty());
    }

    #[test]
    fn events_file_round_trips_and_filters_by_time() {
        let dir = std::env::temp_dir().join(format!("openintel-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("events.jsonl");
        let old = Event {
            polled_at: tuesday_open() - chrono::Duration::hours(2),
            kind: EventKind::MarketState,
            ticker: None,
            summary: "old".into(),
            evidence: vec![],
            source: "clock".into(),
        };
        let new = Event {
            polled_at: tuesday_open(),
            kind: EventKind::PriceMove,
            ticker: Some("AAPL".into()),
            summary: "new".into(),
            evidence: vec!["e".into()],
            source: "yahoo-chart".into(),
        };
        append_events(&path, &[old.clone(), new.clone()]).unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"not json\n")
            .unwrap();
        let (recent, malformed) =
            events_since(&path, tuesday_open() - chrono::Duration::hours(1), 10).unwrap();
        assert_eq!(recent, vec![new]);
        assert_eq!(malformed, 1);
        let (all, _) = events_since(&path, tuesday_open() - chrono::Duration::days(1), 1).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].summary, "new");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(events_since(&path, tuesday_open(), 10).unwrap().0.len(), 0);
    }

    #[tokio::test]
    async fn chatter_poll_is_read_only() {
        let _ = NewsFetch::default();
        let dir =
            std::env::temp_dir().join(format!("openintel-watch-chatter-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let baseline = dir.join("baseline.jsonl");
        let req = ChatterRequest {
            listening: crate::config::listening::ListeningSet::default(),
            hours: 24,
            free_limit: 10,
            x_read_cap: 1,
            baseline_path: Some(baseline.clone()),
            write_baseline: true,
        };
        let feeds: Vec<&dyn ListeningFeed> = Vec::new();
        let mut state = WatchState::default();
        let out = poll_chatter(&req, &feeds, None, &mut state, tuesday_open()).await;
        assert!(out.events.is_empty());
        assert!(
            !baseline.exists(),
            "the watch must never append a baseline line"
        );
    }
}
