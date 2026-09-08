//! Deterministic per-trade risk math: ATR(14) stop, budget-capped
//! size, R-multiple reference levels. Pure and synchronous — a calculator,
//! never an advisor. The clock is stamped by the application layer.

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::domain::error::DomainError;
use crate::domain::values::bar::Bar;

pub const ATR_PERIOD: usize = 14;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Long,
    Short,
}

/// How a size is rounded: whole shares for an equity order, or fractional
/// units for crypto and fractional-share orders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Sizing {
    WholeShares,
    Fractional,
}

impl Sizing {
    pub fn unit(self) -> &'static str {
        match self {
            Sizing::WholeShares => "shares",
            Sizing::Fractional => "units",
        }
    }
}

/// What the caller decides; everything else the frame derives from the bars.
#[derive(Debug, Clone, Copy)]
pub struct FrameSpec {
    pub direction: Direction,
    pub entry: f64,
    pub budget_usd: f64,
    /// Stop distance in ATR multiples (clamped 0.5 to 5).
    pub stop_multiple: f64,
    pub sizing: Sizing,
}

#[derive(Debug, Clone, Serialize)]
pub struct RiskFrame {
    pub ticker: String,
    pub direction: Direction,
    pub entry: f64,
    pub atr: f64,
    pub stop_multiple: f64,
    pub stop: f64,
    pub risk_per_unit: f64,
    /// Whole shares, or fractional units (6 decimals) under `Sizing::Fractional`.
    pub units: f64,
    pub unit: &'static str,
    pub sizing: Sizing,
    /// units × risk_per_unit — the ACTUAL capped loss (≤ budget_usd).
    pub max_loss_usd: f64,
    pub budget_usd: f64,
    /// 1R / 2R / 3R price levels (direction-signed reference exits).
    pub targets: [f64; 3],
    pub notional_usd: f64,
    pub bars_used: usize,
    pub note: Option<String>,
    pub generated_at: DateTime<Utc>,
}

fn fail(message: impl Into<String>) -> DomainError {
    DomainError::SourceFailure {
        name: "risk".into(),
        message: message.into(),
    }
}

/// True ranges for bars[1..] (each needs the previous close).
pub fn true_ranges(bars: &[Bar]) -> Vec<f64> {
    bars.windows(2)
        .map(|w| {
            let prev_close = w[0].close;
            let b = w[1];
            (b.high - b.low)
                .max((b.high - prev_close).abs())
                .max((b.low - prev_close).abs())
        })
        .collect()
}

/// Simple mean of the last `period` true ranges. None if history is too thin.
pub fn atr(bars: &[Bar], period: usize) -> Option<f64> {
    let trs = true_ranges(bars);
    if trs.len() < period || period == 0 {
        return None;
    }
    let tail = &trs[trs.len() - period..];
    Some(tail.iter().sum::<f64>() / period as f64)
}

pub fn frame(
    ticker: &str,
    bars: &[Bar],
    spec: FrameSpec,
    generated_at: DateTime<Utc>,
) -> Result<RiskFrame, DomainError> {
    let FrameSpec {
        direction,
        entry,
        budget_usd,
        stop_multiple,
        sizing,
    } = spec;
    if !(budget_usd.is_finite() && budget_usd > 0.0) {
        return Err(fail("budget must be a positive number"));
    }
    if !(entry.is_finite() && entry > 0.0) {
        return Err(fail("entry must be a positive price"));
    }
    if !(stop_multiple.is_finite() && stop_multiple > 0.0) {
        return Err(fail("stop multiple must be a positive number"));
    }
    let stop_multiple = stop_multiple.clamp(0.5, 5.0);
    if bars
        .iter()
        .any(|b| !(b.high.is_finite() && b.low.is_finite() && b.close.is_finite()))
    {
        return Err(fail("price history contains invalid values"));
    }
    let atr = atr(bars, ATR_PERIOD)
        .ok_or_else(|| fail(format!("not enough history for ATR({ATR_PERIOD})")))?;
    if !(atr.is_finite() && atr > 0.0) {
        return Err(fail("degenerate price history — ATR is zero or invalid"));
    }

    let risk_per_unit = stop_multiple * atr;
    let stop = match direction {
        Direction::Long => entry - risk_per_unit,
        Direction::Short => entry + risk_per_unit,
    };
    if !(stop.is_finite() && stop > 0.0) {
        return Err(fail("stop below zero — use a smaller multiple"));
    }

    let raw_units = budget_usd / risk_per_unit;
    let units = match sizing {
        Sizing::WholeShares => raw_units.floor(),
        Sizing::Fractional => (raw_units * 1e6).floor() / 1e6,
    };
    const MAX_UNITS: f64 = 10_000_000.0; // sanity bound: anything above this is an input error, not a trade
    if units > MAX_UNITS {
        return Err(fail(
            "size implausibly large — check budget and stop multiple",
        ));
    }
    let note = (units == 0.0).then(|| match sizing {
        Sizing::WholeShares => "budget too small for one share at this stop distance".to_string(),
        Sizing::Fractional => {
            "budget too small for a millionth of a unit at this stop distance".to_string()
        }
    });
    let signed = |n: f64| match direction {
        Direction::Long => entry + n * risk_per_unit,
        Direction::Short => entry - n * risk_per_unit,
    };

    let targets = [signed(1.0), signed(2.0), signed(3.0)].map(|t| t.max(0.0));

    Ok(RiskFrame {
        ticker: ticker.to_string(),
        direction,
        entry,
        atr,
        stop_multiple,
        stop,
        risk_per_unit,
        units,
        unit: sizing.unit(),
        sizing,
        max_loss_usd: units * risk_per_unit,
        budget_usd,
        targets,
        notional_usd: units * entry,
        bars_used: bars.len(),
        note,
        generated_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 16, 0, 0, 0).unwrap()
    }

    fn bar(high: f64, low: f64, close: f64) -> Bar {
        Bar {
            date: chrono::NaiveDate::from_ymd_opt(2026, 7, 15).unwrap(),
            open: close,
            high,
            low,
            close,
        }
    }

    /// 16 bars: prev_close 100, then 15 identical bars with TR dominated by
    /// a gap on bar 2 (|high − prev_close| = 8 > high − low = 4).
    fn bars() -> Vec<Bar> {
        let mut v = vec![bar(101.0, 99.0, 100.0)];
        v.push(bar(108.0, 104.0, 106.0)); // gap day: TR = 108-100 = 8
        for _ in 0..14 {
            v.push(bar(108.0, 104.0, 106.0)); // TR = high-low = 4
        }
        v
    }

    #[test]
    fn true_range_counts_gaps() {
        let trs = true_ranges(&bars());
        assert_eq!(trs.len(), 15);
        assert!((trs[0] - 8.0).abs() < 1e-12); // gap day
        assert!((trs[1] - 4.0).abs() < 1e-12);
    }

    #[test]
    fn atr_is_mean_of_last_period() {
        // last 14 TRs are all 4.0 (the gap day falls outside the window)
        assert!((atr(&bars(), 14).unwrap() - 4.0).abs() < 1e-12);
        assert!(atr(&bars()[..14], 14).is_none()); // 13 TRs < 14
    }

    #[test]
    fn long_frame_math() {
        let f = frame(
            "NVDA",
            &bars(),
            FrameSpec {
                direction: Direction::Long,
                entry: 106.0,
                budget_usd: 200.0,
                stop_multiple: 2.0,
                sizing: Sizing::WholeShares,
            },
            at(),
        )
        .unwrap();
        assert!((f.atr - 4.0).abs() < 1e-12);
        assert!((f.risk_per_unit - 8.0).abs() < 1e-12);
        assert!((f.stop - 98.0).abs() < 1e-12);
        assert_eq!(f.units, 25.0); // floor(200 / 8)
        assert_eq!(f.unit, "shares");
        assert!((f.max_loss_usd - 200.0).abs() < 1e-12);
        assert!(f.max_loss_usd <= f.budget_usd);
        assert!((f.targets[0] - 114.0).abs() < 1e-12);
        assert!((f.targets[2] - 130.0).abs() < 1e-12);
        assert!((f.notional_usd - 2650.0).abs() < 1e-12);
        assert!(f.note.is_none());
    }

    #[test]
    fn fractional_sizing_keeps_six_decimals_and_names_the_unit() {
        let f = frame(
            "BTC-USD",
            &bars(),
            FrameSpec {
                direction: Direction::Long,
                entry: 106.0,
                budget_usd: 100.5,
                stop_multiple: 2.0,
                sizing: Sizing::Fractional,
            },
            at(),
        )
        .unwrap();
        assert!((f.units - 12.5625).abs() < 1e-12); // 100.5 / 8, exact
        assert_eq!(f.unit, "units");
        assert!((f.max_loss_usd - 100.5).abs() < 1e-9);
        let tiny = frame(
            "BTC-USD",
            &bars(),
            FrameSpec {
                direction: Direction::Long,
                entry: 106.0,
                budget_usd: 0.0000001,
                stop_multiple: 2.0,
                sizing: Sizing::Fractional,
            },
            at(),
        )
        .unwrap();
        assert_eq!(tiny.units, 0.0);
        assert!(tiny.note.unwrap().contains("millionth"));
    }

    #[test]
    fn short_frame_flips_signs() {
        let f = frame(
            "NVDA",
            &bars(),
            FrameSpec {
                direction: Direction::Short,
                entry: 106.0,
                budget_usd: 100.0,
                stop_multiple: 1.0,
                sizing: Sizing::WholeShares,
            },
            at(),
        )
        .unwrap();
        assert!((f.stop - 110.0).abs() < 1e-12);
        assert!((f.targets[0] - 102.0).abs() < 1e-12);
        assert_eq!(f.units, 25.0); // floor(100 / 4)
    }

    #[test]
    fn short_targets_clamped_at_zero() {
        // Short, entry 10.0, ATR 4, k=2 -> risk_per_share=8.
        // 1R: 10-8=2 (unclamped); 3R: 10-24=-14 (clamped to 0).
        let f = frame(
            "NVDA",
            &bars(),
            FrameSpec {
                direction: Direction::Short,
                entry: 10.0,
                budget_usd: 100.0,
                stop_multiple: 2.0,
                sizing: Sizing::WholeShares,
            },
            at(),
        )
        .unwrap();
        assert!((f.targets[0] - 2.0).abs() < 1e-12); // 1R unclamped
        assert!((f.targets[2] - 0.0).abs() < 1e-12); // 3R clamped to zero
    }

    #[test]
    fn zero_shares_is_valid_with_note_and_max_loss_zero() {
        let f = frame(
            "NVDA",
            &bars(),
            FrameSpec {
                direction: Direction::Long,
                entry: 106.0,
                budget_usd: 5.0,
                stop_multiple: 2.0,
                sizing: Sizing::WholeShares,
            },
            at(),
        )
        .unwrap();
        assert_eq!(f.units, 0.0);
        assert_eq!(f.max_loss_usd, 0.0);
        assert!(f.note.as_deref().unwrap().contains("too small"));
    }

    #[test]
    fn clamps_and_errors() {
        // multiple clamped up from 0.1 to 0.5
        let f = frame(
            "N",
            &bars(),
            FrameSpec {
                direction: Direction::Long,
                entry: 106.0,
                budget_usd: 100.0,
                stop_multiple: 0.1,
                sizing: Sizing::WholeShares,
            },
            at(),
        )
        .unwrap();
        assert!((f.stop_multiple - 0.5).abs() < 1e-12);
        // multiple clamped down from 9 to 5
        let f = frame(
            "N",
            &bars(),
            FrameSpec {
                direction: Direction::Long,
                entry: 106.0,
                budget_usd: 100.0,
                stop_multiple: 9.0,
                sizing: Sizing::WholeShares,
            },
            at(),
        )
        .unwrap();
        assert!((f.stop_multiple - 5.0).abs() < 1e-12);
        assert!(frame(
            "N",
            &bars(),
            FrameSpec {
                direction: Direction::Long,
                entry: 106.0,
                budget_usd: 0.0,
                stop_multiple: 2.0,
                sizing: Sizing::WholeShares
            },
            at()
        )
        .is_err());
        assert!(frame(
            "N",
            &bars(),
            FrameSpec {
                direction: Direction::Long,
                entry: -1.0,
                budget_usd: 100.0,
                stop_multiple: 2.0,
                sizing: Sizing::WholeShares
            },
            at()
        )
        .is_err());
        assert!(frame(
            "N",
            &bars()[..10],
            FrameSpec {
                direction: Direction::Long,
                entry: 106.0,
                budget_usd: 100.0,
                stop_multiple: 2.0,
                sizing: Sizing::WholeShares
            },
            at()
        )
        .is_err());
        // long stop below zero: entry 3, k=5, atr 4 -> stop = -17
        assert!(frame(
            "N",
            &bars(),
            FrameSpec {
                direction: Direction::Long,
                entry: 3.0,
                budget_usd: 100.0,
                stop_multiple: 5.0,
                sizing: Sizing::WholeShares
            },
            at()
        )
        .is_err());
        // flat history -> ATR 0 -> error
        let flat = vec![bar(100.0, 100.0, 100.0); 16];
        assert!(frame(
            "N",
            &flat,
            FrameSpec {
                direction: Direction::Long,
                entry: 100.0,
                budget_usd: 100.0,
                stop_multiple: 2.0,
                sizing: Sizing::WholeShares
            },
            at()
        )
        .is_err());
    }

    #[test]
    fn nan_inputs_error_instead_of_poisoning_output() {
        assert!(frame(
            "N",
            &bars(),
            FrameSpec {
                direction: Direction::Long,
                entry: 106.0,
                budget_usd: 100.0,
                stop_multiple: f64::NAN,
                sizing: Sizing::WholeShares
            },
            at()
        )
        .is_err());
        assert!(frame(
            "N",
            &bars(),
            FrameSpec {
                direction: Direction::Long,
                entry: f64::NAN,
                budget_usd: 100.0,
                stop_multiple: 2.0,
                sizing: Sizing::WholeShares
            },
            at()
        )
        .is_err());
        assert!(frame(
            "N",
            &bars(),
            FrameSpec {
                direction: Direction::Long,
                entry: 106.0,
                budget_usd: f64::NAN,
                stop_multiple: 2.0,
                sizing: Sizing::WholeShares
            },
            at()
        )
        .is_err());
        let mut poisoned = bars();
        poisoned[8] = bar(f64::NAN, 104.0, 106.0);
        assert!(frame(
            "N",
            &poisoned,
            FrameSpec {
                direction: Direction::Long,
                entry: 106.0,
                budget_usd: 100.0,
                stop_multiple: 2.0,
                sizing: Sizing::WholeShares
            },
            at()
        )
        .is_err());
    }

    #[test]
    fn implausible_share_count_errors() {
        // budget astronomically large vs. risk/share of 8 -> shares would exceed the sanity cap
        assert!(frame(
            "N",
            &bars(),
            FrameSpec {
                direction: Direction::Long,
                entry: 106.0,
                budget_usd: 1e12,
                stop_multiple: 2.0,
                sizing: Sizing::WholeShares
            },
            at()
        )
        .is_err());
    }
}
