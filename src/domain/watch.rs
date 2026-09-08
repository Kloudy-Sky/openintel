//! The pure half of the watch loop: given what a poll observed and what
//! earlier polls already reported, decide which events are new. State is a
//! set of keys, so a restarted process re-reports at most one poll's worth.
//! No IO, no clock: `now` is the poll time, injected.

use std::collections::BTreeSet;

use chrono::{DateTime, NaiveDate, Utc};

use crate::domain::clock::{MarketClock, MarketState};
use crate::domain::dip::{headline_gate, is_catalyst_form, GateEvidence, GateStatus};
use crate::domain::values::event::{Event, EventKind};
use crate::domain::values::filing::Filing;
use crate::domain::values::headline::Headline;

pub const DEFAULT_MOVE_THRESHOLD_ATR: f64 = 1.0;

#[derive(Debug, Default)]
pub struct WatchState {
    seen_filings: BTreeSet<String>,
    seen_headlines: BTreeSet<String>,
    emitted_moves: BTreeSet<String>,
    elevated_chatter: BTreeSet<String>,
    last_market_state: Option<MarketState>,
}

/// New catalyst-form filings for `ticker`, each reported once.
pub fn filing_events(
    state: &mut WatchState,
    ticker: &str,
    filings: &[Filing],
    now: DateTime<Utc>,
) -> Vec<Event> {
    let mut out = Vec::new();
    for f in filings.iter().filter(|f| is_catalyst_form(&f.form)) {
        let key = format!("{ticker}|{}|{}", f.form, f.filed_on);
        if state.seen_filings.insert(key) {
            out.push(Event {
                polled_at: now,
                kind: EventKind::Filing,
                ticker: Some(ticker.to_string()),
                summary: format!("{ticker} filed a {} on {}", f.form, f.filed_on),
                evidence: vec![format!("{} filed {}", f.form, f.filed_on)],
                source: "sec-edgar".into(),
            });
        }
    }
    out
}

/// New company-referencing catalyst headlines for `ticker`, judged by the same
/// gate dip uses. Hits that are not clearly about the company stay silent:
/// they cap a verdict, but they are not an event.
pub fn headline_events(
    state: &mut WatchState,
    ticker: &str,
    headlines: &[Headline],
    company_names: &[String],
    now: DateTime<Utc>,
) -> Vec<Event> {
    let (status, evidence) = headline_gate(
        &GateEvidence::Available(headlines.to_vec()),
        ticker,
        company_names,
    );
    if !matches!(status, GateStatus::Fail(_)) {
        return Vec::new();
    }
    let mut out = Vec::new();
    for line in evidence {
        let key = format!("{ticker}|{line}");
        if state.seen_headlines.insert(key) {
            out.push(Event {
                polled_at: now,
                kind: EventKind::CatalystHeadline,
                ticker: Some(ticker.to_string()),
                summary: format!("{ticker}: catalyst headline"),
                evidence: vec![line],
                source: "yahoo-news".into(),
            });
        }
    }
    out
}

/// A quote against its scale: where price is, where it closed, and the ATR
/// that turns the difference into steps.
#[derive(Debug, Clone, Copy)]
pub struct Move {
    pub last: f64,
    pub prior_close: f64,
    pub atr: f64,
    /// Whole ATR multiples that each earn one event.
    pub threshold_atr: f64,
}

/// One event per whole ATR multiple crossed since the prior close, each
/// multiple reported once per session date.
pub fn move_events(
    state: &mut WatchState,
    ticker: &str,
    mv: Move,
    date: NaiveDate,
    now: DateTime<Utc>,
) -> Vec<Event> {
    let Move {
        last,
        prior_close,
        atr,
        threshold_atr,
    } = mv;
    if !(atr.is_finite() && atr > 0.0 && prior_close > 0.0 && threshold_atr > 0.0) {
        return Vec::new();
    }
    let move_atr = (last - prior_close) / atr;
    let steps = (move_atr.abs() / threshold_atr).floor() as u32;
    let mut out = Vec::new();
    for step in 1..=steps {
        let key = format!("{ticker}|{step}|{date}");
        if state.emitted_moves.insert(key) {
            let pct = (last - prior_close) / prior_close * 100.0;
            out.push(Event {
                polled_at: now,
                kind: EventKind::PriceMove,
                ticker: Some(ticker.to_string()),
                summary: format!(
                    "{ticker} {:+.2} ATR from the prior close ({pct:+.1}%)",
                    move_atr
                ),
                evidence: vec![format!(
                    "last {last:.2} vs prior close {prior_close:.2}, ATR(14) {atr:.2}, step {step} of {threshold_atr} ATR"
                )],
                source: "yahoo-chart".into(),
            });
        }
    }
    out
}

/// A chatter velocity flag, once per ticker, platform, and date.
pub fn chatter_event(
    state: &mut WatchState,
    platform: &str,
    ticker: &str,
    elevated: bool,
    ratio: Option<f64>,
    date: NaiveDate,
    now: DateTime<Utc>,
) -> Option<Event> {
    if !elevated {
        return None;
    }
    let key = format!("{ticker}|{platform}|{date}");
    if !state.elevated_chatter.insert(key) {
        return None;
    }
    Some(Event {
        polled_at: now,
        kind: EventKind::ChatterVelocity,
        ticker: Some(ticker.to_string()),
        summary: format!(
            "{ticker} mention velocity elevated on {platform}{}",
            ratio
                .map(|r| format!(" ({r:.1}x baseline)"))
                .unwrap_or_default()
        ),
        evidence: vec![format!("platform {platform}, date {date}")],
        source: format!("chatter-{platform}"),
    })
}

/// The session state changed since the last poll. The first poll only
/// records the state; there is nothing to compare it to.
pub fn market_state_event(
    state: &mut WatchState,
    clock: &MarketClock,
    now: DateTime<Utc>,
) -> Option<Event> {
    let previous = state.last_market_state.replace(clock.state);
    match previous {
        Some(p) if p != clock.state => Some(Event {
            polled_at: now,
            kind: EventKind::MarketState,
            ticker: None,
            summary: format!(
                "{} market {} → {}{}",
                clock.asset_class.as_str(),
                p.as_str(),
                clock.state.as_str(),
                clock
                    .reason
                    .as_ref()
                    .map(|r| format!(" ({r})"))
                    .unwrap_or_default()
            ),
            evidence: vec![format!("clock at {}", clock.now_et)],
            source: "clock".into(),
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::clock::market_clock;
    use crate::domain::values::asset_class::AssetClass;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 8, 14, 0, 0).unwrap()
    }

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn filings_report_catalyst_forms_once() {
        let mut state = WatchState::default();
        let filings = vec![
            Filing {
                form: "8-K".into(),
                filed_on: ymd(2026, 9, 8),
            },
            Filing {
                form: "4".into(),
                filed_on: ymd(2026, 9, 8),
            },
        ];
        let first = filing_events(&mut state, "ADSK", &filings, now());
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].kind, EventKind::Filing);
        assert!(filing_events(&mut state, "ADSK", &filings, now()).is_empty());
    }

    #[test]
    fn headlines_report_company_referencing_hits_once() {
        let mut state = WatchState::default();
        let headlines = vec![Headline {
            title: "Autodesk cuts guidance".into(),
            publisher: "wire".into(),
            published_at: Some(now()),
        }];
        let names = vec!["Autodesk".to_string()];
        let first = headline_events(&mut state, "ADSK", &headlines, &names, now());
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].kind, EventKind::CatalystHeadline);
        assert!(headline_events(&mut state, "ADSK", &headlines, &names, now()).is_empty());

        let roundup = vec![Headline {
            title: "Stocks that cut guidance today".into(),
            publisher: "wire".into(),
            published_at: Some(now()),
        }];
        assert!(headline_events(&mut state, "ADSK", &roundup, &names, now()).is_empty());
    }

    #[test]
    fn moves_step_through_atr_multiples_once_per_day() {
        let mut state = WatchState::default();
        let d = ymd(2026, 9, 8);
        let mv = |last: f64, atr: f64| Move {
            last,
            prior_close: 100.0,
            atr,
            threshold_atr: 1.0,
        };
        assert!(move_events(&mut state, "NVDA", mv(101.0, 4.0), d, now()).is_empty());
        let one = move_events(&mut state, "NVDA", mv(95.0, 4.0), d, now());
        assert_eq!(one.len(), 1);
        assert!(one[0].summary.contains("-1.25 ATR"));
        assert!(move_events(&mut state, "NVDA", mv(94.0, 4.0), d, now()).is_empty());
        let two = move_events(&mut state, "NVDA", mv(91.0, 4.0), d, now());
        assert_eq!(two.len(), 1);
        assert!(two[0].evidence[0].contains("step 2"));
        // a new session date starts the steps over
        assert_eq!(
            move_events(&mut state, "NVDA", mv(91.0, 4.0), ymd(2026, 9, 9), now()).len(),
            2
        );
        assert!(move_events(&mut state, "NVDA", mv(91.0, 0.0), d, now()).is_empty());
    }

    #[test]
    fn chatter_flags_once_per_platform_and_day() {
        let mut state = WatchState::default();
        let d = ymd(2026, 9, 8);
        assert!(chatter_event(&mut state, "bluesky", "TSLA", false, None, d, now()).is_none());
        let e = chatter_event(&mut state, "bluesky", "TSLA", true, Some(3.2), d, now()).unwrap();
        assert!(e.summary.contains("3.2x"));
        assert!(chatter_event(&mut state, "bluesky", "TSLA", true, Some(3.5), d, now()).is_none());
        assert!(chatter_event(&mut state, "reddit", "TSLA", true, None, d, now()).is_some());
    }

    #[test]
    fn market_state_reports_changes_not_the_first_reading() {
        let mut state = WatchState::default();
        let pre = market_clock(Utc.with_ymd_and_hms(2026, 9, 8, 12, 0, 0).unwrap());
        let open = market_clock(Utc.with_ymd_and_hms(2026, 9, 8, 14, 0, 0).unwrap());
        assert_eq!(pre.asset_class, AssetClass::Equity);
        assert!(market_state_event(&mut state, &pre, now()).is_none());
        assert!(market_state_event(&mut state, &pre, now()).is_none());
        let e = market_state_event(&mut state, &open, now()).unwrap();
        assert!(e.summary.contains("pre_market → open"));
        assert!(market_state_event(&mut state, &open, now()).is_none());
    }
}
