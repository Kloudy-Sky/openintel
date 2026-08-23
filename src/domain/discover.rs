//! Pure pieces of discovery: period-extreme levels, deep-slot allocation.
//! Discovery surfaces candidates with evidence attached — it never ranks and
//! never emits a verdict; the catalyst gates it reuses live in `dip`.

use serde::Serialize;

use crate::domain::values::bar::Bar;

/// High/low of the last `days` daily bars, with distance from `last` in ATRs.
/// These are PERIOD EXTREMES — a cheap proxy, not support/resistance; real
/// levels are drawn by the trader.
#[derive(Debug, Clone, Serialize)]
pub struct PeriodExtremes {
    /// Trading days actually covered (may be fewer than requested — reported,
    /// never assumed).
    pub days: usize,
    pub high: f64,
    pub low: f64,
    pub dist_to_high_atr: f64,
    pub dist_to_low_atr: f64,
}

pub fn period_extremes(
    bars: &[Bar],
    last_price: f64,
    atr: f64,
    days: usize,
) -> Option<PeriodExtremes> {
    if bars.is_empty() || days == 0 || !(atr.is_finite() && atr > 0.0) {
        return None;
    }
    let window = &bars[bars.len().saturating_sub(days)..];
    let high = window.iter().map(|b| b.high).fold(f64::MIN, f64::max);
    let low = window.iter().map(|b| b.low).fold(f64::MAX, f64::min);
    if !(high.is_finite() && low.is_finite()) {
        return None;
    }
    Some(PeriodExtremes {
        days: window.len(),
        high,
        low,
        dist_to_high_atr: (high - last_price) / atr,
        dist_to_low_atr: (last_price - low) / atr,
    })
}

/// Spread `total` deep-annotation slots across screen groups round-robin, in
/// group order, capped by each group's size. No ranking — provider order
/// within a group is preserved by the caller.
pub fn allocate_deep(group_sizes: &[usize], total: usize) -> Vec<usize> {
    let mut alloc = vec![0usize; group_sizes.len()];
    let mut remaining = total;
    while remaining > 0 {
        let mut progressed = false;
        for (i, size) in group_sizes.iter().enumerate() {
            if remaining == 0 {
                break;
            }
            if alloc[i] < *size {
                alloc[i] += 1;
                remaining -= 1;
                progressed = true;
            }
        }
        if !progressed {
            break;
        }
    }
    alloc
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn bar(day: u32, low: f64, high: f64) -> Bar {
        Bar {
            date: NaiveDate::from_ymd_opt(2026, 1, 1)
                .unwrap()
                .checked_add_days(chrono::Days::new(day as u64))
                .unwrap(),
            open: (low + high) / 2.0,
            high,
            low,
            close: (low + high) / 2.0,
        }
    }

    #[test]
    fn extremes_report_actual_span_and_atr_distances() {
        let bars: Vec<Bar> = (0..10)
            .map(|d| bar(d, 90.0 + d as f64, 100.0 + d as f64))
            .collect();
        // Window of last 5: lows 95..99, highs 105..109.
        let e = period_extremes(&bars, 100.0, 2.0, 5).unwrap();
        assert_eq!(e.days, 5);
        assert_eq!(e.high, 109.0);
        assert_eq!(e.low, 95.0);
        assert!((e.dist_to_high_atr - 4.5).abs() < 1e-12);
        assert!((e.dist_to_low_atr - 2.5).abs() < 1e-12);

        // Ask for more days than exist: span is honest.
        let e = period_extremes(&bars, 100.0, 2.0, 250).unwrap();
        assert_eq!(e.days, 10);
    }

    #[test]
    fn extremes_refuse_bad_inputs() {
        assert!(period_extremes(&[], 100.0, 2.0, 5).is_none());
        assert!(period_extremes(&[bar(0, 90.0, 100.0)], 100.0, 0.0, 5).is_none());
    }

    #[test]
    fn deep_allocation_is_round_robin_and_capped() {
        assert_eq!(allocate_deep(&[5, 5, 5], 9), vec![3, 3, 3]);
        assert_eq!(allocate_deep(&[1, 5, 5], 9), vec![1, 4, 4]);
        assert_eq!(allocate_deep(&[0, 0, 2], 9), vec![0, 0, 2]);
        assert_eq!(allocate_deep(&[4, 4], 3), vec![2, 1]);
        assert_eq!(allocate_deep(&[], 5), Vec::<usize>::new());
    }
}
