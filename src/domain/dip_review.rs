//! Pure grading math for the dip journal: forward returns per entry, stats
//! per verdict bucket, and a score↔return correlation. This is the feedback
//! loop that decides whether the v0 dip weights deserve trust — until it
//! shows edge on a meaningful sample, `high_confidence` stays "clean setup",
//! not "profitable setup". Synchronous, no IO, no clock.

use chrono::NaiveDate;
use serde::Serialize;

use crate::domain::dip::{Session, Verdict};
use crate::domain::values::bar::Bar;

/// Grading horizons in trading days.
pub const HORIZONS: [usize; 3] = [1, 5, 10];

/// Raw percentage returns from the journaled price to the close of the Nth
/// trading bar after the scan date. None = not enough forward bars yet.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct ForwardReturns {
    pub d1: Option<f64>,
    pub d5: Option<f64>,
    pub d10: Option<f64>,
}

impl ForwardReturns {
    pub fn get(&self, horizon: usize) -> Option<f64> {
        match horizon {
            1 => self.d1,
            5 => self.d5,
            10 => self.d10,
            _ => None,
        }
    }
}

/// Forward returns for an entry against ascending daily bars. The first bar
/// strictly after `entry_date` is trading day 1.
pub fn forward_returns(entry_date: NaiveDate, entry_price: f64, bars: &[Bar]) -> ForwardReturns {
    if !(entry_price.is_finite() && entry_price > 0.0) {
        return ForwardReturns::default();
    }
    let Some(first_fwd) = bars.iter().position(|b| b.date > entry_date) else {
        return ForwardReturns::default();
    };
    let ret = |n: usize| {
        bars.get(first_fwd + n - 1)
            .map(|b| (b.close - entry_price) / entry_price * 100.0)
    };
    ForwardReturns {
        d1: ret(1),
        d5: ret(5),
        d10: ret(10),
    }
}

/// One journal entry joined with its forward outcomes.
#[derive(Debug, Clone, Serialize)]
pub struct GradedEntry {
    pub ticker: String,
    pub scanned_on: NaiveDate,
    pub session: Session,
    pub verdict: Verdict,
    pub score: f64,
    pub entry_price: f64,
    /// Raw forward returns in percent.
    pub returns: ForwardReturns,
    /// Returns minus the index proxy's same-horizon return (market-adjusted).
    pub excess: ForwardReturns,
}

#[derive(Debug, Clone, Serialize)]
pub struct HorizonStats {
    pub n: usize,
    pub mean_pct: f64,
    pub median_pct: f64,
    /// Fraction of entries with a positive return.
    pub win_rate: f64,
}

pub fn horizon_stats(values: &[f64]) -> Option<HorizonStats> {
    if values.is_empty() {
        return None;
    }
    let n = values.len();
    let mean = values.iter().sum::<f64>() / n as f64;
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = if n % 2 == 1 {
        sorted[n / 2]
    } else {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    };
    let wins = values.iter().filter(|v| **v > 0.0).count();
    Some(HorizonStats {
        n,
        mean_pct: mean,
        median_pct: median,
        win_rate: wins as f64 / n as f64,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct HorizonSet {
    pub d1: Option<HorizonStats>,
    pub d5: Option<HorizonStats>,
    pub d10: Option<HorizonStats>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VerdictBucket {
    pub verdict: Verdict,
    pub n: usize,
    pub raw: HorizonSet,
    pub excess: HorizonSet,
}

fn horizon_set(
    entries: &[&GradedEntry],
    pick: impl Fn(&GradedEntry, usize) -> Option<f64>,
) -> HorizonSet {
    let collect = |h: usize| {
        let vals: Vec<f64> = entries.iter().filter_map(|e| pick(e, h)).collect();
        horizon_stats(&vals)
    };
    HorizonSet {
        d1: collect(1),
        d5: collect(5),
        d10: collect(10),
    }
}

/// Bucket graded entries by verdict, strongest claim first.
pub fn aggregate(entries: &[GradedEntry]) -> Vec<VerdictBucket> {
    [Verdict::HighConfidence, Verdict::Watch, Verdict::NoSetup]
        .into_iter()
        .filter_map(|verdict| {
            let bucket: Vec<&GradedEntry> =
                entries.iter().filter(|e| e.verdict == verdict).collect();
            if bucket.is_empty() {
                return None;
            }
            Some(VerdictBucket {
                verdict,
                n: bucket.len(),
                raw: horizon_set(&bucket, |e, h| e.returns.get(h)),
                excess: horizon_set(&bucket, |e, h| e.excess.get(h)),
            })
        })
        .collect()
}

/// Pearson correlation; None below 3 pairs or with zero variance.
pub fn pearson(xs: &[f64], ys: &[f64]) -> Option<f64> {
    let n = xs.len();
    if n != ys.len() || n < 3 {
        return None;
    }
    let nf = n as f64;
    let mx = xs.iter().sum::<f64>() / nf;
    let my = ys.iter().sum::<f64>() / nf;
    let (mut cov, mut vx, mut vy) = (0.0, 0.0, 0.0);
    for (x, y) in xs.iter().zip(ys) {
        cov += (x - mx) * (y - my);
        vx += (x - mx).powi(2);
        vy += (y - my).powi(2);
    }
    if vx == 0.0 || vy == 0.0 {
        return None;
    }
    Some(cov / (vx.sqrt() * vy.sqrt()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 8, day).unwrap()
    }

    fn bar(day: u32, close: f64) -> Bar {
        Bar {
            date: d(day),
            open: close,
            high: close + 1.0,
            low: close - 1.0,
            close,
        }
    }

    #[test]
    fn forward_returns_index_from_first_bar_after_entry() {
        // entry on the 10th at 100; bars for 10th..21st (weekdays irrelevant here)
        let bars: Vec<Bar> = (10..=21).map(|i| bar(i, 100.0 + i as f64)).collect();
        let fr = forward_returns(d(10), 100.0, &bars);
        assert!((fr.d1.unwrap() - 11.0).abs() < 1e-9); // close 111 on the 11th
        assert!((fr.d5.unwrap() - 15.0).abs() < 1e-9); // close 115 on the 15th
        assert!((fr.d10.unwrap() - 20.0).abs() < 1e-9);

        // too recent: only 2 forward bars -> d5/d10 pending
        let fr = forward_returns(d(19), 100.0, &bars);
        assert!(fr.d1.is_some());
        assert!(fr.d5.is_none() && fr.d10.is_none());

        // stale/no coverage: entry after all bars
        let fr = forward_returns(d(25), 100.0, &bars);
        assert!(fr.d1.is_none());

        // degenerate price
        let fr = forward_returns(d(10), 0.0, &bars);
        assert!(fr.d1.is_none());
    }

    #[test]
    fn stats_mean_median_win() {
        let s = horizon_stats(&[1.0, -2.0, 3.0, 4.0]).unwrap();
        assert_eq!(s.n, 4);
        assert!((s.mean_pct - 1.5).abs() < 1e-9);
        assert!((s.median_pct - 2.0).abs() < 1e-9);
        assert!((s.win_rate - 0.75).abs() < 1e-9);
        assert!(horizon_stats(&[]).is_none());
    }

    #[test]
    fn aggregate_buckets_by_verdict_in_order() {
        let entry = |verdict, r1: f64| GradedEntry {
            ticker: "T".into(),
            scanned_on: d(10),
            session: Session::PostClose,
            verdict,
            score: 50.0,
            entry_price: 100.0,
            returns: ForwardReturns {
                d1: Some(r1),
                d5: None,
                d10: None,
            },
            excess: ForwardReturns::default(),
        };
        let entries = vec![
            entry(Verdict::Watch, -1.0),
            entry(Verdict::HighConfidence, 2.0),
            entry(Verdict::Watch, 3.0),
        ];
        let buckets = aggregate(&entries);
        assert_eq!(buckets.len(), 2); // no no_setup bucket
        assert_eq!(buckets[0].verdict, Verdict::HighConfidence);
        assert_eq!(buckets[0].n, 1);
        assert_eq!(buckets[1].verdict, Verdict::Watch);
        assert!((buckets[1].raw.d1.as_ref().unwrap().mean_pct - 1.0).abs() < 1e-9);
        assert!(buckets[1].raw.d5.is_none());
    }

    #[test]
    fn pearson_known_values() {
        // perfectly correlated
        assert!((pearson(&[1.0, 2.0, 3.0], &[2.0, 4.0, 6.0]).unwrap() - 1.0).abs() < 1e-9);
        // perfectly anti-correlated
        assert!((pearson(&[1.0, 2.0, 3.0], &[3.0, 2.0, 1.0]).unwrap() + 1.0).abs() < 1e-9);
        // guards
        assert!(pearson(&[1.0, 2.0], &[1.0, 2.0]).is_none()); // n < 3
        assert!(pearson(&[1.0, 1.0, 1.0], &[1.0, 2.0, 3.0]).is_none()); // zero variance
    }
}
