//! IO edge for the trade journal: append/read the JSONL, stamp timestamps and
//! trade ids, fetch bars for the review. The domain fold and grading stay pure.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use futures::StreamExt;
use serde::Serialize;

use crate::application::dip::SPX_PROXY;
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::ports::bar_source::BarSource;
use crate::domain::trade_journal::{
    find_duplicate, fold, open_event, CloseReason, Instrument, RiskSnapshot, Trade, TradeEvent,
};
use crate::domain::trade_review::{aggregate, grade, Bucket, GradedTrade, MIN_OVERALL_SAMPLE};

const CONCURRENCY: usize = 4;

fn fail(message: impl Into<String>) -> DomainError {
    DomainError::SourceFailure {
        name: "trade_journal".into(),
        message: message.into(),
    }
}

/// `~/.openintel/trade_journal.jsonl`, or None when no home dir resolves.
pub fn default_path() -> Option<PathBuf> {
    std::env::home_dir().map(|h| h.join(".openintel").join("trade_journal.jsonl"))
}

pub fn default_path_or_err() -> Result<PathBuf, DomainError> {
    default_path().ok_or_else(|| fail("cannot resolve a home directory for the journal"))
}

fn append(path: &Path, event: &TradeEvent) -> Result<(), DomainError> {
    use std::io::Write as _;
    let json = serde_json::to_string(event).map_err(|e| fail(e.to_string()))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| fail(e.to_string()))?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| fail(e.to_string()))?;
    writeln!(file, "{json}").map_err(|e| fail(e.to_string()))
}

/// Tolerant read: unparseable lines are counted, never fatal.
fn read_events(path: &Path) -> (Vec<TradeEvent>, usize) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return (Vec::new(), 0);
    };
    let mut events = Vec::new();
    let mut skipped = 0usize;
    for line in content.lines().filter(|l| !l.trim().is_empty()) {
        match serde_json::from_str::<TradeEvent>(line) {
            Ok(e) => events.push(e),
            Err(_) => skipped += 1,
        }
    }
    (events, skipped)
}

#[derive(Debug, Clone)]
pub struct LogTradeRequest {
    pub source: String,
    pub instrument: Instrument,
    pub qty: f64,
    pub entry: f64,
    pub thesis: String,
    pub setup_tag: String,
    pub planned_stop: f64,
    pub planned_target: Option<f64>,
    /// Both or neither: risk USD and the wallet size right now.
    pub risk_usd: Option<f64>,
    pub wallet_usd: Option<f64>,
    /// Skip the same-day duplicate guard.
    pub allow_duplicate: bool,
}

#[derive(Debug, Serialize)]
pub struct LogOutcome {
    pub trade_id: String,
    pub logged: bool,
    /// Set when the duplicate guard fired: the id of the existing trade.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duplicate_of: Option<String>,
}

/// Append an `opened` event. The trade id is `{SYMBOL}-{yyyymmdd}-{n}`,
/// numbered per symbol per day; a same-day open matching instrument + qty +
/// entry is rejected as a retry unless `allow_duplicate`.
pub fn log_trade(
    path: &Path,
    req: LogTradeRequest,
    now: DateTime<Utc>,
) -> Result<LogOutcome, DomainError> {
    let (events, _) = read_events(path);
    let outcome = fold(&events);
    let instrument = req.instrument.normalized()?;

    if !req.allow_duplicate {
        if let Some(existing) = find_duplicate(
            &outcome.trades,
            &instrument,
            req.qty,
            req.entry,
            now.date_naive(),
        ) {
            return Ok(LogOutcome {
                trade_id: existing.id.clone(),
                logged: false,
                duplicate_of: Some(existing.id.clone()),
            });
        }
    }

    let risk = match (req.risk_usd, req.wallet_usd) {
        (Some(r), Some(w)) => Some(RiskSnapshot::new(r, w)?),
        (None, None) => None,
        _ => {
            return Err(fail(
                "risk_usd and wallet_usd go together — pass both or neither",
            ))
        }
    };

    let day = now.format("%Y%m%d");
    let prefix = format!("{}-{day}-", instrument.symbol());
    let n = outcome
        .trades
        .iter()
        .filter(|t| t.id.starts_with(&prefix))
        .count()
        + 1;
    let trade_id = format!("{prefix}{n}");

    let event = open_event(
        trade_id.clone(),
        now,
        req.source,
        instrument,
        req.qty,
        req.entry,
        req.thesis,
        req.setup_tag,
        req.planned_stop,
        req.planned_target,
        risk,
    )?;
    append(path, &event)?;
    Ok(LogOutcome {
        trade_id,
        logged: true,
        duplicate_of: None,
    })
}

#[derive(Debug, Clone)]
pub enum TradeUpdate {
    Amend {
        note: String,
        new_stop: Option<f64>,
        new_target: Option<f64>,
    },
    Close {
        exit: f64,
        reason: CloseReason,
        note: Option<String>,
    },
}

/// Append an `amended` or `closed` event after checking the trade exists and
/// is still open.
pub fn update_trade(
    path: &Path,
    trade_id: &str,
    update: TradeUpdate,
    now: DateTime<Utc>,
) -> Result<(), DomainError> {
    let (events, _) = read_events(path);
    let outcome = fold(&events);
    let trade = outcome
        .trades
        .iter()
        .find(|t| t.id == trade_id)
        .ok_or_else(|| fail(format!("no trade {trade_id} in the journal")))?;
    if !trade.is_open() {
        return Err(fail(format!("trade {trade_id} is already closed")));
    }
    let event = match update {
        TradeUpdate::Amend {
            note,
            new_stop,
            new_target,
        } => {
            if note.trim().is_empty() {
                return Err(fail("an amendment needs a note — say why"));
            }
            TradeEvent::Amended {
                trade_id: trade_id.to_string(),
                at: now,
                note: note.trim().to_string(),
                new_stop,
                new_target,
            }
        }
        TradeUpdate::Close { exit, reason, note } => {
            // Zero is legal: an out-of-the-money option expires worthless.
            if !(exit.is_finite() && exit >= 0.0) {
                return Err(fail("exit must be a non-negative price"));
            }
            TradeEvent::Closed {
                trade_id: trade_id.to_string(),
                at: now,
                exit,
                reason,
                note,
            }
        }
    };
    append(path, &event)
}

#[derive(Debug, Serialize)]
pub struct PositionsReport {
    pub journal_path: String,
    pub open: Vec<Trade>,
    pub closed_count: usize,
    pub skipped_lines: usize,
    pub errors: Vec<String>,
}

/// The cross-session memory: every open trade with its frozen thesis and plan.
pub fn open_positions(path: &Path) -> PositionsReport {
    let (events, skipped_lines) = read_events(path);
    let outcome = fold(&events);
    let (open, closed): (Vec<Trade>, Vec<Trade>) =
        outcome.trades.into_iter().partition(|t| t.is_open());
    PositionsReport {
        journal_path: path.display().to_string(),
        open,
        closed_count: closed.len(),
        skipped_lines,
        errors: outcome.errors,
    }
}

#[derive(Debug, Serialize)]
pub struct TradeReviewReport {
    pub generated_at: DateTime<Utc>,
    pub journal_path: String,
    pub trades_total: usize,
    pub closed: usize,
    pub open: usize,
    pub by_setup: Vec<Bucket>,
    pub by_instrument: Vec<Bucket>,
    pub trades: Vec<GradedTrade>,
    pub skipped_lines: usize,
    pub errors: Vec<String>,
    pub notes: Vec<String>,
}

/// Grade the whole journal. Equity, crypto, and forex trades get forward
/// returns from Yahoo bars (SPY-adjusted for equities only); options grade on
/// realized P&L plus the underlying's direction.
pub async fn review_trades(
    path: &Path,
    bars_src: &dyn BarSource,
    now: DateTime<Utc>,
) -> Result<TradeReviewReport, DomainError> {
    let (events, skipped_lines) = read_events(path);
    if events.is_empty() {
        return Err(fail(format!(
            "no journal at {} — log a trade first",
            path.display()
        )));
    }
    let outcome = fold(&events);
    let mut notes = Vec::new();
    let mut errors = outcome.errors;

    // Bars for every instrument's symbol (the underlying for options), plus SPY.
    let mut symbols: HashSet<String> = HashSet::new();
    for t in &outcome.trades {
        symbols.insert(t.instrument.symbol().to_string());
    }
    let mut symbols: Vec<String> = symbols.into_iter().collect();
    symbols.sort();
    symbols.push(SPX_PROXY.to_string());

    let fetched: Vec<(
        String,
        Result<Vec<crate::domain::values::bar::Bar>, DomainError>,
    )> = futures::stream::iter(symbols.into_iter().map(|symbol| async move {
        let result = match Ticker::parse(&symbol) {
            Ok(t) => bars_src.bars(&t).await,
            Err(e) => Err(e),
        };
        (symbol, result)
    }))
    .buffer_unordered(CONCURRENCY)
    .collect()
    .await;

    let mut history = HashMap::new();
    for (symbol, result) in fetched {
        match result {
            Ok(bars) => {
                history.insert(symbol, bars);
            }
            Err(e) => errors.push(format!("{symbol}: bars unavailable: {e}")),
        }
    }
    let spy_bars = history.get(SPX_PROXY).map(|b| b.as_slice());

    let graded: Vec<GradedTrade> = outcome
        .trades
        .iter()
        .map(|t| {
            let bars = history.get(t.instrument.symbol()).map(|b| b.as_slice());
            grade(t, bars, spy_bars)
        })
        .collect();

    let closed = graded.iter().filter(|g| !g.open).count();
    if closed < MIN_OVERALL_SAMPLE {
        notes.push(format!(
            "only {closed} closed trades — conclusions need {MIN_OVERALL_SAMPLE}; \
             treat every number below as anecdote, not evidence"
        ));
    }

    let (by_setup, by_instrument) = aggregate(&graded);
    Ok(TradeReviewReport {
        generated_at: now,
        journal_path: path.display().to_string(),
        trades_total: graded.len(),
        closed,
        open: graded.len() - closed,
        by_setup,
        by_instrument,
        trades: graded,
        skipped_lines,
        errors,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::values::bar::Bar;
    use async_trait::async_trait;
    use chrono::{NaiveDate, TimeZone};

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 21, 15, 0, 0).unwrap()
    }

    fn tmp_journal(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("openintel-tj-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir); // a prior failed run must not leak state in
        dir.join("trade_journal.jsonl")
    }

    fn equity_req(entry: f64) -> LogTradeRequest {
        LogTradeRequest {
            source: "cli".into(),
            instrument: Instrument::Equity {
                ticker: "NVDA".into(),
            },
            qty: 10.0,
            entry,
            thesis: "support bounce".into(),
            setup_tag: "sr-support-bounce".into(),
            planned_stop: entry - 5.0,
            planned_target: None,
            risk_usd: Some(50.0),
            wallet_usd: Some(5000.0),
            allow_duplicate: false,
        }
    }

    struct FixedBars(Vec<Bar>);

    #[async_trait]
    impl BarSource for FixedBars {
        async fn bars(&self, _t: &Ticker) -> Result<Vec<Bar>, DomainError> {
            Ok(self.0.clone())
        }
    }

    #[test]
    fn log_ids_number_per_symbol_per_day_and_guard_duplicates() {
        let path = tmp_journal("log");
        let a = log_trade(&path, equity_req(100.0), now()).unwrap();
        assert_eq!(a.trade_id, "NVDA-20260821-1");
        assert!(a.logged);

        let dup = log_trade(&path, equity_req(100.0), now()).unwrap();
        assert!(!dup.logged);
        assert_eq!(dup.duplicate_of, Some("NVDA-20260821-1".into()));

        let b = log_trade(&path, equity_req(101.0), now()).unwrap();
        assert_eq!(b.trade_id, "NVDA-20260821-2");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn update_flows_amend_then_close_then_reject() {
        let path = tmp_journal("update");
        let id = log_trade(&path, equity_req(100.0), now()).unwrap().trade_id;
        update_trade(
            &path,
            &id,
            TradeUpdate::Amend {
                note: "trailing".into(),
                new_stop: Some(98.0),
                new_target: None,
            },
            now(),
        )
        .unwrap();
        update_trade(
            &path,
            &id,
            TradeUpdate::Close {
                exit: 108.0,
                reason: CloseReason::Discretion,
                note: None,
            },
            now(),
        )
        .unwrap();
        let again = update_trade(
            &path,
            &id,
            TradeUpdate::Close {
                exit: 109.0,
                reason: CloseReason::Discretion,
                note: None,
            },
            now(),
        );
        assert!(again.unwrap_err().to_string().contains("already closed"));
        assert!(update_trade(
            &path,
            "GHOST-1",
            TradeUpdate::Amend {
                note: "x".into(),
                new_stop: None,
                new_target: None,
            },
            now()
        )
        .is_err());

        let positions = open_positions(&path);
        assert!(positions.open.is_empty());
        assert_eq!(positions.closed_count, 1);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn lowercase_symbols_normalize_and_cannot_dodge_the_duplicate_guard() {
        let path = tmp_journal("normalize");
        let mut req = equity_req(100.0);
        req.instrument = Instrument::Equity {
            ticker: "nvda".into(),
        };
        let a = log_trade(&path, req, now()).unwrap();
        assert_eq!(a.trade_id, "NVDA-20260821-1");

        let dup = log_trade(&path, equity_req(100.0), now()).unwrap();
        assert!(!dup.logged);

        let mut junk = equity_req(100.0);
        junk.instrument = Instrument::Equity {
            ticker: "$$$".into(),
        };
        assert!(log_trade(&path, junk, now()).is_err());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn worthless_expiry_closes_at_zero() {
        let path = tmp_journal("expiry");
        let id = log_trade(&path, equity_req(100.0), now()).unwrap().trade_id;
        update_trade(
            &path,
            &id,
            TradeUpdate::Close {
                exit: 0.0,
                reason: CloseReason::Expiry,
                note: None,
            },
            now(),
        )
        .unwrap();
        let positions = open_positions(&path);
        assert!(positions.open.is_empty());
        assert_eq!(positions.closed_count, 1);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn risk_fields_go_together() {
        let path = tmp_journal("risk");
        let mut req = equity_req(100.0);
        req.wallet_usd = None;
        assert!(log_trade(&path, req, now()).is_err());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn review_grades_closed_trade_and_notes_small_sample() {
        let path = tmp_journal("review");
        let id = log_trade(&path, equity_req(100.0), now()).unwrap().trade_id;
        update_trade(
            &path,
            &id,
            TradeUpdate::Close {
                exit: 110.0,
                reason: CloseReason::Target,
                note: None,
            },
            now(),
        )
        .unwrap();

        let bars: Vec<Bar> = (22..=28)
            .map(|d| Bar {
                date: NaiveDate::from_ymd_opt(2026, 8, d).unwrap(),
                open: 100.0,
                high: 111.0,
                low: 99.0,
                close: 104.0,
            })
            .collect();
        let report = review_trades(&path, &FixedBars(bars), now()).await.unwrap();
        assert_eq!(report.closed, 1);
        assert_eq!(report.trades[0].realized_r, Some(2.0));
        assert!(report.trades[0].forward.is_some());
        assert!(report.notes.iter().any(|n| n.contains("anecdote")));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn review_of_missing_journal_is_clean_error() {
        let path = tmp_journal("missing");
        let err = review_trades(&path, &FixedBars(vec![]), now()).await;
        assert!(err.unwrap_err().to_string().contains("log a trade first"));
    }
}
