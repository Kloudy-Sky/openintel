//! Deterministic per-trade risk math: ATR(14) stop, budget-capped whole-share
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

#[derive(Debug, Clone, Serialize)]
pub struct RiskFrame {
    pub ticker: String,
    pub direction: Direction,
    pub entry: f64,
    pub atr: f64,
    pub stop_multiple: f64,
    pub stop: f64,
    pub risk_per_share: f64,
    pub shares: u64,
    /// shares × risk_per_share — the ACTUAL capped loss (≤ budget_usd).
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
    direction: Direction,
    entry: f64,
    budget_usd: f64,
    stop_multiple: f64,
    generated_at: DateTime<Utc>,
) -> Result<RiskFrame, DomainError> {
    if !(budget_usd.is_finite() && budget_usd > 0.0) {
        return Err(fail("budget must be a positive number"));
    }
    if !(entry.is_finite() && entry > 0.0) {
        return Err(fail("entry must be a positive price"));
    }
    let stop_multiple = stop_multiple.clamp(0.5, 5.0);
    let atr = atr(bars, ATR_PERIOD)
        .ok_or_else(|| fail(format!("not enough history for ATR({ATR_PERIOD})")))?;
    if atr <= 0.0 {
        return Err(fail("flat price history — ATR is zero"));
    }

    let risk_per_share = stop_multiple * atr;
    let stop = match direction {
        Direction::Long => entry - risk_per_share,
        Direction::Short => entry + risk_per_share,
    };
    if stop <= 0.0 {
        return Err(fail("stop below zero — use a smaller multiple"));
    }

    let shares = (budget_usd / risk_per_share).floor() as u64;
    let note =
        (shares == 0).then(|| "budget too small for one share at this stop distance".to_string());
    let signed = |n: f64| match direction {
        Direction::Long => entry + n * risk_per_share,
        Direction::Short => entry - n * risk_per_share,
    };

    Ok(RiskFrame {
        ticker: ticker.to_string(),
        direction,
        entry,
        atr,
        stop_multiple,
        stop,
        risk_per_share,
        shares,
        max_loss_usd: shares as f64 * risk_per_share,
        budget_usd,
        targets: [signed(1.0), signed(2.0), signed(3.0)],
        notional_usd: shares as f64 * entry,
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
        Bar { high, low, close }
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
        let f = frame("NVDA", &bars(), Direction::Long, 106.0, 200.0, 2.0, at()).unwrap();
        assert!((f.atr - 4.0).abs() < 1e-12);
        assert!((f.risk_per_share - 8.0).abs() < 1e-12);
        assert!((f.stop - 98.0).abs() < 1e-12);
        assert_eq!(f.shares, 25); // floor(200 / 8)
        assert!((f.max_loss_usd - 200.0).abs() < 1e-12);
        assert!(f.max_loss_usd <= f.budget_usd);
        assert!((f.targets[0] - 114.0).abs() < 1e-12);
        assert!((f.targets[2] - 130.0).abs() < 1e-12);
        assert!((f.notional_usd - 2650.0).abs() < 1e-12);
        assert!(f.note.is_none());
    }

    #[test]
    fn short_frame_flips_signs() {
        let f = frame("NVDA", &bars(), Direction::Short, 106.0, 100.0, 1.0, at()).unwrap();
        assert!((f.stop - 110.0).abs() < 1e-12);
        assert!((f.targets[0] - 102.0).abs() < 1e-12);
        assert_eq!(f.shares, 25); // floor(100 / 4)
    }

    #[test]
    fn zero_shares_is_valid_with_note_and_max_loss_zero() {
        let f = frame("NVDA", &bars(), Direction::Long, 106.0, 5.0, 2.0, at()).unwrap();
        assert_eq!(f.shares, 0);
        assert_eq!(f.max_loss_usd, 0.0);
        assert!(f.note.as_deref().unwrap().contains("too small"));
    }

    #[test]
    fn clamps_and_errors() {
        // multiple clamped up from 0.1 to 0.5
        let f = frame("N", &bars(), Direction::Long, 106.0, 100.0, 0.1, at()).unwrap();
        assert!((f.stop_multiple - 0.5).abs() < 1e-12);
        // multiple clamped down from 9 to 5
        let f = frame("N", &bars(), Direction::Long, 106.0, 100.0, 9.0, at()).unwrap();
        assert!((f.stop_multiple - 5.0).abs() < 1e-12);
        assert!(frame("N", &bars(), Direction::Long, 106.0, 0.0, 2.0, at()).is_err());
        assert!(frame("N", &bars(), Direction::Long, -1.0, 100.0, 2.0, at()).is_err());
        assert!(frame("N", &bars()[..10], Direction::Long, 106.0, 100.0, 2.0, at()).is_err());
        // long stop below zero: entry 3, k=5, atr 4 -> stop = -17
        assert!(frame("N", &bars(), Direction::Long, 3.0, 100.0, 5.0, at()).is_err());
        // flat history -> ATR 0 -> error
        let flat = vec![bar(100.0, 100.0, 100.0); 16];
        assert!(frame("N", &flat, Direction::Long, 100.0, 100.0, 2.0, at()).is_err());
    }
}
