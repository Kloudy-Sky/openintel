//! Grade folded trades: R-multiples against the original plan, discipline
//! metrics (did the exit honor the plan?), and forward returns wherever bars
//! exist (equity, crypto, forex), SPY-adjusted for equities only.
//! Pure — bars are fetched at the application edge and passed in.
//!
//! Honesty gates: overall conclusions need n ≥ 30 closed trades, per-bucket
//! stats need n ≥ 10; below that the review says "insufficient sample" and
//! shows no number. Options and crypto grade on realized P&L only (historical
//! option prices aren't available keyless; crypto has no bar path yet).

use serde::Serialize;

use crate::domain::dip_review::{forward_returns, horizon_stats, ForwardReturns, HorizonStats};
use crate::domain::trade_journal::{CloseReason, Instrument, OptionKind, Trade};
use crate::domain::values::bar::Bar;

pub const MIN_OVERALL_SAMPLE: usize = 30;
pub const MIN_BUCKET_SAMPLE: usize = 10;

#[derive(Debug, Clone, Serialize)]
pub struct GradedTrade {
    pub id: String,
    pub instrument: String,
    pub instrument_kind: &'static str,
    pub setup_tag: String,
    pub open: bool,
    /// Realized R vs the ORIGINAL stop — amendments never soften the grade.
    pub realized_r: Option<f64>,
    pub realized_pnl_usd: Option<f64>,
    pub close_reason: Option<CloseReason>,
    /// 1/5/10-day forward returns from entry where bars exist; SPY-adjusted for equities.
    pub forward: Option<ForwardReturns>,
    pub forward_vs_spy: Option<ForwardReturns>,
    /// Options only: did the underlying move the thesis direction by day 5?
    pub underlying_with_thesis: Option<bool>,
    pub stop_widened: bool,
    /// Plan-vs-actual flags, e.g. "exited 0.8R below the original stop".
    pub discipline: Vec<String>,
}

/// Grade one trade. `underlying_bars` are ascending daily bars for the
/// instrument's symbol (underlying for options); None when unavailable.
pub fn grade(
    trade: &Trade,
    underlying_bars: Option<&[Bar]>,
    spy_bars: Option<&[Bar]>,
) -> GradedTrade {
    let mut discipline = Vec::new();
    let rpu = trade.risk_per_unit();

    let (realized_r, realized_pnl_usd, close_reason) = match &trade.close {
        None => (None, None, None),
        Some(c) => {
            let r = (c.exit - trade.entry) / rpu;
            let pnl = (c.exit - trade.entry) * trade.qty;
            if c.exit < trade.original_stop {
                let overrun = (trade.original_stop - c.exit) / rpu;
                discipline.push(format!("exited {overrun:.1}R below the original stop"));
            }
            (Some(r), Some(pnl), Some(c.reason))
        }
    };
    if trade.stop_widened {
        discipline.push("stop was widened after entry".into());
    }

    let entry_date = trade.opened_at.date_naive();
    let is_equity = matches!(trade.instrument, Instrument::Equity { .. });
    let has_bar_path = matches!(
        trade.instrument,
        Instrument::Equity { .. } | Instrument::Crypto { .. } | Instrument::Forex { .. }
    );
    let forward = match (has_bar_path, underlying_bars) {
        (true, Some(bars)) => Some(forward_returns(entry_date, trade.entry, bars)),
        _ => None,
    };
    let forward_vs_spy = match (&forward, spy_bars) {
        (Some(fwd), Some(spy)) if is_equity => {
            let base = spy
                .iter()
                .find(|b| b.date >= entry_date)
                .map(|b| forward_returns(entry_date, b.close, spy));
            base.map(|b| ForwardReturns {
                d1: sub(fwd.d1, b.d1),
                d5: sub(fwd.d5, b.d5),
                d10: sub(fwd.d10, b.d10),
            })
        }
        _ => None,
    };

    let underlying_with_thesis = match (&trade.instrument, underlying_bars) {
        (Instrument::Option { kind, .. }, Some(bars)) => bars
            .iter()
            .find(|b| b.date >= entry_date)
            .map(|base| forward_returns(entry_date, base.close, bars))
            .and_then(|f| f.d5)
            .map(|d5| match kind {
                OptionKind::Call => d5 > 0.0,
                OptionKind::Put => d5 < 0.0,
            }),
        _ => None,
    };

    GradedTrade {
        id: trade.id.clone(),
        instrument: trade.instrument.describe(),
        instrument_kind: trade.instrument.kind_label(),
        setup_tag: trade.setup_tag.clone(),
        open: trade.is_open(),
        realized_r,
        realized_pnl_usd,
        close_reason,
        forward,
        forward_vs_spy,
        underlying_with_thesis,
        stop_widened: trade.stop_widened,
        discipline,
    }
}

fn sub(a: Option<f64>, b: Option<f64>) -> Option<f64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a - b),
        _ => None,
    }
}

/// Aggregate stats for one bucket (a setup tag or an instrument kind).
/// Numbers appear only at n ≥ MIN_BUCKET_SAMPLE closed trades.
#[derive(Debug, Clone, Serialize)]
pub struct Bucket {
    pub key: String,
    pub trades: usize,
    pub closed: usize,
    pub open: usize,
    pub stops_widened: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub win_rate_pct: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_realized_r: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub d5_forward: Option<HorizonStats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

fn bucket(key: String, graded: &[&GradedTrade]) -> Bucket {
    let closed: Vec<&&GradedTrade> = graded.iter().filter(|g| !g.open).collect();
    let rs: Vec<f64> = closed.iter().filter_map(|g| g.realized_r).collect();
    let d5s: Vec<f64> = graded
        .iter()
        .filter_map(|g| g.forward.as_ref()?.d5)
        .collect();
    let enough = closed.len() >= MIN_BUCKET_SAMPLE;
    Bucket {
        key,
        trades: graded.len(),
        closed: closed.len(),
        open: graded.len() - closed.len(),
        stops_widened: graded.iter().filter(|g| g.stop_widened).count(),
        win_rate_pct: enough.then(|| {
            let wins = closed
                .iter()
                .filter(|g| g.realized_pnl_usd.unwrap_or(0.0) > 0.0)
                .count();
            wins as f64 / closed.len() as f64 * 100.0
        }),
        avg_realized_r: (enough && !rs.is_empty())
            .then(|| rs.iter().sum::<f64>() / rs.len() as f64),
        d5_forward: if enough { horizon_stats(&d5s) } else { None },
        note: (!enough).then(|| {
            format!(
                "insufficient sample — {} closed, need {MIN_BUCKET_SAMPLE}",
                closed.len()
            )
        }),
    }
}

/// Bucket graded trades by setup tag and by instrument kind.
pub fn aggregate(graded: &[GradedTrade]) -> (Vec<Bucket>, Vec<Bucket>) {
    let mut tags: Vec<String> = graded.iter().map(|g| g.setup_tag.clone()).collect();
    tags.sort();
    tags.dedup();
    let by_tag = tags
        .into_iter()
        .map(|tag| {
            let members: Vec<&GradedTrade> = graded.iter().filter(|g| g.setup_tag == tag).collect();
            bucket(tag, &members)
        })
        .collect();

    let mut kinds: Vec<&'static str> = graded.iter().map(|g| g.instrument_kind).collect();
    kinds.sort();
    kinds.dedup();
    let by_kind = kinds
        .into_iter()
        .map(|kind| {
            let members: Vec<&GradedTrade> = graded
                .iter()
                .filter(|g| g.instrument_kind == kind)
                .collect();
            bucket(kind.to_string(), &members)
        })
        .collect();
    (by_tag, by_kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::trade_journal::{fold, open_event, RiskSnapshot, TradeEvent};
    use chrono::{NaiveDate, TimeZone, Utc};

    fn trade(id: &str, closed_at: Option<f64>) -> Trade {
        let mut events = vec![open_event(
            id.into(),
            Utc.with_ymd_and_hms(2026, 8, 3, 14, 0, 0).unwrap(),
            "agent".into(),
            Instrument::Equity {
                ticker: "NVDA".into(),
            },
            10.0,
            100.0,
            "thesis".into(),
            "sr-support-bounce".into(),
            95.0,
            None,
            Some(RiskSnapshot::new(50.0, 5000.0).unwrap()),
        )
        .unwrap()];
        if let Some(exit) = closed_at {
            events.push(TradeEvent::Closed {
                trade_id: id.into(),
                at: Utc.with_ymd_and_hms(2026, 8, 10, 20, 0, 0).unwrap(),
                exit,
                reason: CloseReason::Discretion,
                note: None,
            });
        }
        fold(&events).trades.remove(0)
    }

    fn bars_from(day: u32, closes: &[f64]) -> Vec<Bar> {
        closes
            .iter()
            .enumerate()
            .map(|(i, c)| Bar {
                date: NaiveDate::from_ymd_opt(2026, 8, day + i as u32).unwrap(),
                open: *c,
                high: *c + 1.0,
                low: *c - 1.0,
                close: *c,
            })
            .collect()
    }

    #[test]
    fn realized_r_uses_original_stop() {
        let g = grade(&trade("NVDA-1", Some(110.0)), None, None);
        assert_eq!(g.realized_r, Some(2.0));
        assert_eq!(g.realized_pnl_usd, Some(100.0));
        assert!(g.discipline.is_empty());
    }

    #[test]
    fn stop_overrun_is_reported_in_r() {
        let g = grade(&trade("NVDA-1", Some(92.5)), None, None);
        assert_eq!(g.realized_r, Some(-1.5));
        assert_eq!(g.discipline, vec!["exited 0.5R below the original stop"]);
    }

    #[test]
    fn equity_forward_returns_computed_from_entry_date() {
        // entry Aug 3 @ 100; bars Aug 4..: d1 = 102
        let bars = bars_from(4, &[102.0, 103.0, 104.0, 105.0, 106.0]);
        let g = grade(&trade("NVDA-1", None), Some(&bars), None);
        let fwd = g.forward.unwrap();
        assert_eq!(fwd.d1, Some(2.0));
        assert_eq!(fwd.d5, Some(6.0));
        assert!(g.open);
    }

    #[test]
    fn option_grades_underlying_direction_not_forward_returns() {
        let opened = open_event(
            "TSLA-1".into(),
            Utc.with_ymd_and_hms(2026, 8, 3, 14, 0, 0).unwrap(),
            "agent".into(),
            Instrument::Option {
                underlying: "TSLA".into(),
                strike: 250.0,
                expiry: NaiveDate::from_ymd_opt(2026, 9, 18).unwrap(),
                kind: OptionKind::Put,
            },
            1.0,
            4.20,
            "rejection at resistance".into(),
            "sr-resistance-reject".into(),
            2.10,
            None,
            None,
        )
        .unwrap();
        let t = fold(&[opened]).trades.remove(0);
        let bars = bars_from(3, &[240.0, 238.0, 236.0, 235.0, 233.0, 231.0]);
        let g = grade(&t, Some(&bars), None);
        assert!(g.forward.is_none());
        assert_eq!(g.underlying_with_thesis, Some(true));
    }

    #[test]
    fn buckets_stay_silent_below_sample_floor() {
        let graded: Vec<GradedTrade> = (0..5)
            .map(|i| grade(&trade(&format!("NVDA-{i}"), Some(110.0)), None, None))
            .collect();
        let (by_tag, by_kind) = aggregate(&graded);
        assert_eq!(by_tag.len(), 1);
        assert!(by_tag[0].win_rate_pct.is_none());
        assert!(by_tag[0].note.as_ref().unwrap().contains("insufficient"));
        assert_eq!(by_kind[0].key, "equity");
    }

    #[test]
    fn buckets_report_at_sample_floor() {
        let graded: Vec<GradedTrade> = (0..MIN_BUCKET_SAMPLE)
            .map(|i| {
                let exit = if i % 2 == 0 { 110.0 } else { 96.0 };
                grade(&trade(&format!("NVDA-{i}"), Some(exit)), None, None)
            })
            .collect();
        let (by_tag, _) = aggregate(&graded);
        assert_eq!(by_tag[0].win_rate_pct, Some(50.0));
        assert!(by_tag[0].avg_realized_r.is_some());
        assert!(by_tag[0].note.is_none());
    }
}
