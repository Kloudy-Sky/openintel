//! Append-only trade journal: events in, folded trades out. A trade is a fold
//! over its `opened` / `amended` / `closed` events; the entry thesis and the
//! original plan are frozen at open and can never be rewritten. Pure and
//! synchronous — timestamps and trade ids are stamped at the application edge.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::domain::error::DomainError;

fn fail(message: impl Into<String>) -> DomainError {
    DomainError::SourceFailure {
        name: "trade_journal".into(),
        message: message.into(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OptionKind {
    Call,
    Put,
}

/// What was traded. Matches the instruments a Robinhood agentic account can
/// hold (long equities, long options, crypto). Schema is deliberately free of
/// OpenIntel-specific fields so the journal can stand alone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Instrument {
    Equity {
        ticker: String,
    },
    Option {
        underlying: String,
        strike: f64,
        expiry: NaiveDate,
        kind: OptionKind,
    },
    Crypto {
        symbol: String,
    },
}

impl Instrument {
    /// The symbol used for trade ids and bar lookups (underlying for options).
    pub fn symbol(&self) -> &str {
        match self {
            Instrument::Equity { ticker } => ticker,
            Instrument::Option { underlying, .. } => underlying,
            Instrument::Crypto { symbol } => symbol,
        }
    }

    pub fn kind_label(&self) -> &'static str {
        match self {
            Instrument::Equity { .. } => "equity",
            Instrument::Option { .. } => "option",
            Instrument::Crypto { .. } => "crypto",
        }
    }

    /// Canonical form for persistence: ticker-backed symbols validated and
    /// uppercased via `Ticker::parse` (so a lowercase retry can't slip past
    /// the duplicate guard, and review can always fetch bars), crypto trimmed
    /// and uppercased.
    pub fn normalized(self) -> Result<Instrument, DomainError> {
        use crate::domain::entities::ticker::Ticker;
        Ok(match self {
            Instrument::Equity { ticker } => Instrument::Equity {
                ticker: Ticker::parse(&ticker)?.as_str().to_string(),
            },
            Instrument::Option {
                underlying,
                strike,
                expiry,
                kind,
            } => Instrument::Option {
                underlying: Ticker::parse(&underlying)?.as_str().to_string(),
                strike,
                expiry,
                kind,
            },
            Instrument::Crypto { symbol } => {
                let symbol = symbol.trim().to_ascii_uppercase();
                if symbol.is_empty() {
                    return Err(fail("crypto symbol is required"));
                }
                Instrument::Crypto { symbol }
            }
        })
    }

    pub fn describe(&self) -> String {
        match self {
            Instrument::Equity { ticker } => ticker.clone(),
            Instrument::Option {
                underlying,
                strike,
                expiry,
                kind,
            } => {
                let k = match kind {
                    OptionKind::Call => "call",
                    OptionKind::Put => "put",
                };
                format!("{underlying} {strike} {k} {expiry}")
            }
            Instrument::Crypto { symbol } => symbol.clone(),
        }
    }
}

/// Risk snapshot taken at open — wallet size as it was that moment, so the
/// percentage never goes stale.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct RiskSnapshot {
    pub risk_usd: f64,
    pub wallet_usd: f64,
    pub pct_of_wallet: f64,
}

impl RiskSnapshot {
    pub fn new(risk_usd: f64, wallet_usd: f64) -> Result<Self, DomainError> {
        if !(risk_usd.is_finite() && risk_usd > 0.0 && wallet_usd.is_finite() && wallet_usd > 0.0) {
            return Err(fail("risk and wallet must be positive numbers"));
        }
        Ok(Self {
            risk_usd,
            wallet_usd,
            pct_of_wallet: risk_usd / wallet_usd * 100.0,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CloseReason {
    Stop,
    Target,
    Discretion,
    Expiry,
}

/// One journal line. `opened` freezes the thesis and plan; `amended` appends
/// (never rewrites); `closed` ends the trade.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum TradeEvent {
    Opened {
        trade_id: String,
        at: DateTime<Utc>,
        /// Who logged it: "agent", "cli", later "dip_scan".
        source: String,
        instrument: Instrument,
        qty: f64,
        entry: f64,
        thesis: String,
        setup_tag: String,
        /// Price of the traded instrument (premium for options), strictly
        /// below entry — long-only, so risk-per-unit is always positive.
        planned_stop: f64,
        planned_target: Option<f64>,
        risk: Option<RiskSnapshot>,
    },
    Amended {
        trade_id: String,
        at: DateTime<Utc>,
        note: String,
        new_stop: Option<f64>,
        new_target: Option<f64>,
    },
    Closed {
        trade_id: String,
        at: DateTime<Utc>,
        exit: f64,
        reason: CloseReason,
        note: Option<String>,
    },
}

impl TradeEvent {
    pub fn trade_id(&self) -> &str {
        match self {
            TradeEvent::Opened { trade_id, .. }
            | TradeEvent::Amended { trade_id, .. }
            | TradeEvent::Closed { trade_id, .. } => trade_id,
        }
    }
}

/// Validated constructor for an `opened` event. All plan fields are required
/// except the target — no entry without a stop.
#[allow(clippy::too_many_arguments)]
pub fn open_event(
    trade_id: String,
    at: DateTime<Utc>,
    source: String,
    instrument: Instrument,
    qty: f64,
    entry: f64,
    thesis: String,
    setup_tag: String,
    planned_stop: f64,
    planned_target: Option<f64>,
    risk: Option<RiskSnapshot>,
) -> Result<TradeEvent, DomainError> {
    if !(qty.is_finite() && qty > 0.0) {
        return Err(fail("qty must be a positive number"));
    }
    if !(entry.is_finite() && entry > 0.0) {
        return Err(fail("entry must be a positive price"));
    }
    if !(planned_stop.is_finite() && planned_stop > 0.0 && planned_stop < entry) {
        return Err(fail(
            "planned stop must be a positive price strictly below entry (long-only)",
        ));
    }
    if let Some(t) = planned_target {
        if !(t.is_finite() && t > entry) {
            return Err(fail("planned target must be above entry"));
        }
    }
    if thesis.trim().is_empty() {
        return Err(fail("thesis is required — no entry without a written why"));
    }
    if setup_tag.trim().is_empty() {
        return Err(fail("setup_tag is required (e.g. sr-support-bounce)"));
    }
    Ok(TradeEvent::Opened {
        trade_id,
        at,
        source,
        instrument,
        qty,
        entry,
        thesis: thesis.trim().to_string(),
        setup_tag: setup_tag.trim().to_string(),
        planned_stop,
        planned_target,
        risk,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct Amendment {
    pub at: DateTime<Utc>,
    pub note: String,
    pub new_stop: Option<f64>,
    pub new_target: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Close {
    pub at: DateTime<Utc>,
    pub exit: f64,
    pub reason: CloseReason,
    pub note: Option<String>,
}

/// A trade folded from its events. Original plan fields are the frozen open
/// values; `current_*` reflect amendments.
#[derive(Debug, Clone, Serialize)]
pub struct Trade {
    pub id: String,
    pub opened_at: DateTime<Utc>,
    pub source: String,
    pub instrument: Instrument,
    pub qty: f64,
    pub entry: f64,
    pub thesis: String,
    pub setup_tag: String,
    pub original_stop: f64,
    pub original_target: Option<f64>,
    pub current_stop: f64,
    pub current_target: Option<f64>,
    /// True if any amendment moved the stop below its original level.
    pub stop_widened: bool,
    pub risk: Option<RiskSnapshot>,
    pub amendments: Vec<Amendment>,
    pub close: Option<Close>,
}

impl Trade {
    pub fn is_open(&self) -> bool {
        self.close.is_none()
    }

    /// entry − original stop, always positive by open validation.
    pub fn risk_per_unit(&self) -> f64 {
        self.entry - self.original_stop
    }
}

#[derive(Debug, Default)]
pub struct FoldOutcome {
    pub trades: Vec<Trade>,
    /// Per-event problems (unknown id, double close, amend-after-close).
    /// A bad event never sinks the fold.
    pub errors: Vec<String>,
}

/// Fold events (in file order) into trades. Order within a trade is the append
/// order; events referencing unknown or closed trades become errors.
pub fn fold(events: &[TradeEvent]) -> FoldOutcome {
    let mut out = FoldOutcome::default();
    for event in events {
        match event {
            TradeEvent::Opened {
                trade_id,
                at,
                source,
                instrument,
                qty,
                entry,
                thesis,
                setup_tag,
                planned_stop,
                planned_target,
                risk,
            } => {
                if out.trades.iter().any(|t| &t.id == trade_id) {
                    out.errors
                        .push(format!("duplicate open for trade {trade_id}"));
                    continue;
                }
                out.trades.push(Trade {
                    id: trade_id.clone(),
                    opened_at: *at,
                    source: source.clone(),
                    instrument: instrument.clone(),
                    qty: *qty,
                    entry: *entry,
                    thesis: thesis.clone(),
                    setup_tag: setup_tag.clone(),
                    original_stop: *planned_stop,
                    original_target: *planned_target,
                    current_stop: *planned_stop,
                    current_target: *planned_target,
                    stop_widened: false,
                    risk: *risk,
                    amendments: Vec::new(),
                    close: None,
                });
            }
            TradeEvent::Amended {
                trade_id,
                at,
                note,
                new_stop,
                new_target,
            } => match out.trades.iter_mut().find(|t| &t.id == trade_id) {
                None => out
                    .errors
                    .push(format!("amend for unknown trade {trade_id}")),
                Some(t) if t.close.is_some() => out
                    .errors
                    .push(format!("amend after close for trade {trade_id}")),
                Some(t) => {
                    if let Some(s) = new_stop {
                        if *s < t.original_stop {
                            t.stop_widened = true;
                        }
                        t.current_stop = *s;
                    }
                    if new_target.is_some() {
                        t.current_target = *new_target;
                    }
                    t.amendments.push(Amendment {
                        at: *at,
                        note: note.clone(),
                        new_stop: *new_stop,
                        new_target: *new_target,
                    });
                }
            },
            TradeEvent::Closed {
                trade_id,
                at,
                exit,
                reason,
                note,
            } => match out.trades.iter_mut().find(|t| &t.id == trade_id) {
                None => out
                    .errors
                    .push(format!("close for unknown trade {trade_id}")),
                Some(t) if t.close.is_some() => out
                    .errors
                    .push(format!("second close for trade {trade_id}")),
                Some(t) => {
                    t.close = Some(Close {
                        at: *at,
                        exit: *exit,
                        reason: *reason,
                        note: note.clone(),
                    });
                }
            },
        }
    }
    out
}

/// Retry guard: an open matching instrument + qty + entry on the same UTC day
/// is almost certainly the same real trade logged twice.
pub fn find_duplicate<'a>(
    trades: &'a [Trade],
    instrument: &Instrument,
    qty: f64,
    entry: f64,
    day: NaiveDate,
) -> Option<&'a Trade> {
    trades.iter().find(|t| {
        &t.instrument == instrument
            && t.qty == qty
            && t.entry == entry
            && t.opened_at.date_naive() == day
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 21, h, 0, 0).unwrap()
    }

    fn equity(ticker: &str) -> Instrument {
        Instrument::Equity {
            ticker: ticker.into(),
        }
    }

    fn opened(id: &str) -> TradeEvent {
        open_event(
            id.into(),
            at(14),
            "agent".into(),
            equity("NVDA"),
            10.0,
            100.0,
            "bounce off weekly support at 98".into(),
            "sr-support-bounce".into(),
            95.0,
            Some(110.0),
            Some(RiskSnapshot::new(50.0, 5000.0).unwrap()),
        )
        .unwrap()
    }

    #[test]
    fn open_validation_rejects_bad_plans() {
        let bad_stop = open_event(
            "x".into(),
            at(14),
            "agent".into(),
            equity("NVDA"),
            10.0,
            100.0,
            "t".into(),
            "tag".into(),
            100.0, // stop == entry
            None,
            None,
        );
        assert!(bad_stop.is_err());
        let no_thesis = open_event(
            "x".into(),
            at(14),
            "agent".into(),
            equity("NVDA"),
            10.0,
            100.0,
            "  ".into(),
            "tag".into(),
            95.0,
            None,
            None,
        );
        assert!(no_thesis.is_err());
        let target_below_entry = open_event(
            "x".into(),
            at(14),
            "agent".into(),
            equity("NVDA"),
            10.0,
            100.0,
            "t".into(),
            "tag".into(),
            95.0,
            Some(99.0),
            None,
        );
        assert!(target_below_entry.is_err());
    }

    #[test]
    fn fold_builds_trade_and_tracks_amendments() {
        let events = vec![
            opened("NVDA-1"),
            TradeEvent::Amended {
                trade_id: "NVDA-1".into(),
                at: at(15),
                note: "trailing up after 1R".into(),
                new_stop: Some(100.0),
                new_target: None,
            },
            TradeEvent::Closed {
                trade_id: "NVDA-1".into(),
                at: at(20),
                exit: 108.0,
                reason: CloseReason::Discretion,
                note: None,
            },
        ];
        let out = fold(&events);
        assert!(out.errors.is_empty());
        let t = &out.trades[0];
        assert_eq!(t.original_stop, 95.0);
        assert_eq!(t.current_stop, 100.0);
        assert!(!t.stop_widened);
        assert!(!t.is_open());
        assert_eq!(t.close.as_ref().unwrap().exit, 108.0);
        assert_eq!(t.risk_per_unit(), 5.0);
    }

    #[test]
    fn widened_stop_is_flagged() {
        let events = vec![
            opened("NVDA-1"),
            TradeEvent::Amended {
                trade_id: "NVDA-1".into(),
                at: at(15),
                note: "giving it room".into(),
                new_stop: Some(92.0),
                new_target: None,
            },
        ];
        let out = fold(&events);
        assert!(out.trades[0].stop_widened);
    }

    #[test]
    fn bad_references_become_errors_not_panics() {
        let events = vec![
            TradeEvent::Closed {
                trade_id: "GHOST".into(),
                at: at(14),
                exit: 1.0,
                reason: CloseReason::Stop,
                note: None,
            },
            opened("NVDA-1"),
            TradeEvent::Closed {
                trade_id: "NVDA-1".into(),
                at: at(15),
                exit: 96.0,
                reason: CloseReason::Stop,
                note: None,
            },
            TradeEvent::Closed {
                trade_id: "NVDA-1".into(),
                at: at(16),
                exit: 97.0,
                reason: CloseReason::Stop,
                note: None,
            },
            TradeEvent::Amended {
                trade_id: "NVDA-1".into(),
                at: at(17),
                note: "too late".into(),
                new_stop: None,
                new_target: None,
            },
        ];
        let out = fold(&events);
        assert_eq!(out.trades.len(), 1);
        assert_eq!(out.errors.len(), 3);
        assert_eq!(out.trades[0].close.as_ref().unwrap().exit, 96.0);
    }

    #[test]
    fn duplicate_guard_matches_same_day_same_shape() {
        let out = fold(&[opened("NVDA-1")]);
        assert!(find_duplicate(
            &out.trades,
            &equity("NVDA"),
            10.0,
            100.0,
            at(14).date_naive()
        )
        .is_some());
        assert!(find_duplicate(
            &out.trades,
            &equity("NVDA"),
            10.0,
            101.0,
            at(14).date_naive()
        )
        .is_none());
    }

    #[test]
    fn events_round_trip_through_jsonl() {
        let e = opened("NVDA-1");
        let line = serde_json::to_string(&e).unwrap();
        let back: TradeEvent = serde_json::from_str(&line).unwrap();
        assert_eq!(back.trade_id(), "NVDA-1");
        let opt = Instrument::Option {
            underlying: "TSLA".into(),
            strike: 250.0,
            expiry: NaiveDate::from_ymd_opt(2026, 9, 18).unwrap(),
            kind: OptionKind::Call,
        };
        let line = serde_json::to_string(&opt).unwrap();
        let back: Instrument = serde_json::from_str(&line).unwrap();
        assert_eq!(back, opt);
        assert_eq!(back.describe(), "TSLA 250 call 2026-09-18");
    }
}
