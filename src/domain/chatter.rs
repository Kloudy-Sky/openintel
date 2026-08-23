//! Pure chatter math: cashtag extraction, mention counting, and velocity
//! against a baseline journal. Honesty gates throughout: velocity claims need
//! ≥ MIN_VELOCITY_MENTIONS today and ≥ MIN_BASELINE_DAYS of prior baseline
//! from an unchanged listening set — below that, raw counts only, labeled.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::domain::entities::ticker::Ticker;

/// A ticker must be cashtagged at least this often (across a platform's scan)
/// to appear in the report at all — single mentions are noise.
pub const MIN_REPORT_MENTIONS: usize = 3;
/// Velocity (a ratio claim) needs more: this many mentions today…
pub const MIN_VELOCITY_MENTIONS: usize = 5;
/// …and this many prior baseline days for the same platform + listening set.
pub const MIN_BASELINE_DAYS: usize = 3;
/// "Elevated" = today's mention share at least this multiple of the baseline
/// mean share.
pub const ELEVATED_RATIO: f64 = 3.0;

/// Valid cashtags in `text` ($NVDA → NVDA), deduplicated per call — a ticker
/// counts once per post no matter how many times one author repeats it.
pub fn cashtags(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut found: Vec<String> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && bytes[end].is_ascii_alphabetic() && end - start < 6 {
                end += 1;
            }
            let boundary_ok = end == bytes.len() || !bytes[end].is_ascii_alphanumeric();
            if end > start && end - start <= 5 && boundary_ok {
                if let Ok(t) = Ticker::parse(&text[start..end]) {
                    let sym = t.as_str().to_string();
                    if !found.contains(&sym) {
                        found.push(sym);
                    }
                }
            }
            i = end;
        } else {
            i += 1;
        }
    }
    found
}

#[derive(Debug, Clone, Serialize)]
pub struct MentionCounts {
    pub total_posts: usize,
    pub counts: BTreeMap<String, usize>,
}

/// Count cashtag mentions across posts — once per ticker per post.
pub fn count_mentions(texts: &[&str]) -> MentionCounts {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for text in texts {
        for sym in cashtags(text) {
            *counts.entry(sym).or_insert(0) += 1;
        }
    }
    MentionCounts {
        total_posts: texts.len(),
        counts,
    }
}

/// One appended baseline line: what one platform's scan counted on one day.
/// `fingerprint` identifies the listening set the counts came from — velocity
/// against a changed set is flagged, never silently claimed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaselineLine {
    pub date: NaiveDate,
    pub platform: String,
    pub fingerprint: String,
    pub total_posts: usize,
    pub counts: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Velocity {
    pub mentions: usize,
    /// mentions ÷ total posts scanned — set-size-independent.
    pub share: f64,
    pub baseline_days: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline_share_mean: Option<f64>,
    /// today's share ÷ baseline mean share. None until the honesty gates pass.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ratio: Option<f64>,
    pub elevated: bool,
    pub flags: Vec<String>,
}

/// Velocity for one ticker on one platform. `history` is prior days' lines
/// for the same platform (any fingerprint — mismatches are flagged); today's
/// line must not be included.
pub fn velocity(
    ticker: &str,
    today: &MentionCounts,
    history: &[BaselineLine],
    fingerprint: &str,
) -> Velocity {
    let mentions = today.counts.get(ticker).copied().unwrap_or(0);
    let share = if today.total_posts > 0 {
        mentions as f64 / today.total_posts as f64
    } else {
        0.0
    };
    let mut flags = Vec::new();

    let matching: Vec<&BaselineLine> = history
        .iter()
        .filter(|l| l.fingerprint == fingerprint && l.total_posts > 0)
        .collect();
    if matching.len() < history.len() {
        flags.push("listening set changed within the baseline window".into());
    }
    // One line per day: a re-run same day would double-weight it.
    let mut days: Vec<NaiveDate> = matching.iter().map(|l| l.date).collect();
    days.sort();
    days.dedup();
    let baseline_days = days.len();

    let baseline_share_mean = (baseline_days > 0).then(|| {
        let sum: f64 = days
            .iter()
            .map(|d| {
                // Last line per day wins.
                let line = matching.iter().rev().find(|l| l.date == *d).unwrap();
                line.counts.get(ticker).copied().unwrap_or(0) as f64 / line.total_posts as f64
            })
            .sum();
        sum / baseline_days as f64
    });

    let ratio = match (baseline_share_mean, mentions, baseline_days) {
        (Some(mean), m, d) if m >= MIN_VELOCITY_MENTIONS && d >= MIN_BASELINE_DAYS => {
            if mean > 0.0 {
                Some(share / mean)
            } else {
                flags.push("first appearance — no baseline to compare against".into());
                None
            }
        }
        _ => {
            if baseline_days < MIN_BASELINE_DAYS {
                flags.push(format!(
                    "baseline building — {baseline_days}/{MIN_BASELINE_DAYS} prior days; raw counts only"
                ));
            } else if mentions < MIN_VELOCITY_MENTIONS {
                flags.push(format!(
                    "below velocity floor ({mentions} < {MIN_VELOCITY_MENTIONS} mentions); raw counts only"
                ));
            }
            None
        }
    };
    let elevated = ratio.is_some_and(|r| r >= ELEVATED_RATIO);

    Velocity {
        mentions,
        share,
        baseline_days,
        baseline_share_mean,
        ratio,
        elevated,
        flags,
    }
}

/// The "before the chart" read: chatter elevated while the tape is quiet.
/// None when any input is missing — never a claim without evidence.
pub fn before_the_chart(
    elevated: bool,
    change_pct: Option<f64>,
    rvol: Option<f64>,
) -> Option<bool> {
    match (change_pct, rvol) {
        (Some(change), Some(rvol)) => Some(elevated && change.abs() < 2.0 && rvol < 1.5),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 8, day).unwrap()
    }

    fn line(day: u32, fp: &str, total: usize, pairs: &[(&str, usize)]) -> BaselineLine {
        BaselineLine {
            date: d(day),
            platform: "x".into(),
            fingerprint: fp.into(),
            total_posts: total,
            counts: pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        }
    }

    #[test]
    fn cashtags_validate_and_dedupe() {
        assert_eq!(
            cashtags("Long $NVDA and $nvda, watching $TSLA. $5k gain, $TOOLONGX no"),
            vec!["NVDA", "TSLA"]
        );
        assert!(cashtags("price is $50 and 100$").is_empty());
        assert_eq!(cashtags("$BRK.B maybe"), vec!["BRK"]); // cashtag grammar stops at '.'
    }

    #[test]
    fn mentions_count_once_per_post() {
        let m = count_mentions(&["$NVDA $NVDA $NVDA to the moon", "$NVDA dip", "no tags"]);
        assert_eq!(m.total_posts, 3);
        assert_eq!(m.counts["NVDA"], 2);
    }

    #[test]
    fn velocity_gates_on_baseline_days_and_mentions() {
        let today = count_mentions(&["$NVDA", "$NVDA", "$NVDA", "$NVDA", "$NVDA", "x"]);
        // Only 2 baseline days -> no ratio, building flag.
        let v = velocity(
            "NVDA",
            &today,
            &[
                line(18, "fp", 100, &[("NVDA", 2)]),
                line(19, "fp", 100, &[("NVDA", 2)]),
            ],
            "fp",
        );
        assert_eq!(v.mentions, 5);
        assert!(v.ratio.is_none());
        assert!(v.flags.iter().any(|f| f.contains("baseline building")));

        // 3 days -> ratio appears: share 5/6 vs mean 0.02.
        let history = [
            line(17, "fp", 100, &[("NVDA", 2)]),
            line(18, "fp", 100, &[("NVDA", 2)]),
            line(19, "fp", 100, &[("NVDA", 2)]),
        ];
        let v = velocity("NVDA", &today, &history, "fp");
        let ratio = v.ratio.unwrap();
        assert!((ratio - (5.0 / 6.0) / 0.02).abs() < 1e-9);
        assert!(v.elevated);

        // Below the mention floor -> raw counts only.
        let sparse = count_mentions(&["$NVDA", "$NVDA", "$NVDA", "a", "b", "c"]);
        let v = velocity("NVDA", &sparse, &history, "fp");
        assert!(v.ratio.is_none());
        assert!(v.flags.iter().any(|f| f.contains("below velocity floor")));
    }

    #[test]
    fn changed_fingerprint_is_flagged_and_excluded() {
        let today = count_mentions(&["$NVDA", "$NVDA", "$NVDA", "$NVDA", "$NVDA"]);
        let history = [
            line(17, "old", 100, &[("NVDA", 50)]),
            line(18, "fp", 100, &[("NVDA", 2)]),
            line(19, "fp", 100, &[("NVDA", 2)]),
        ];
        let v = velocity("NVDA", &today, &history, "fp");
        assert_eq!(v.baseline_days, 2);
        assert!(v.flags.iter().any(|f| f.contains("listening set changed")));
    }

    #[test]
    fn same_day_rerun_counts_once_with_last_line_winning() {
        let today = count_mentions(&["$NVDA", "$NVDA", "$NVDA", "$NVDA", "$NVDA"]);
        let history = [
            line(17, "fp", 100, &[("NVDA", 2)]),
            line(18, "fp", 100, &[("NVDA", 2)]),
            line(19, "fp", 10, &[("NVDA", 9)]),
            line(19, "fp", 100, &[("NVDA", 2)]), // re-run: this one counts
        ];
        let v = velocity("NVDA", &today, &history, "fp");
        assert_eq!(v.baseline_days, 3);
        assert!((v.baseline_share_mean.unwrap() - 0.02).abs() < 1e-12);
    }

    #[test]
    fn first_appearance_has_no_ratio() {
        let today = count_mentions(&["$NEWCO", "$NEWCO", "$NEWCO", "$NEWCO", "$NEWCO"]);
        let history = [
            line(17, "fp", 100, &[]),
            line(18, "fp", 100, &[]),
            line(19, "fp", 100, &[]),
        ];
        let v = velocity("NEWCO", &today, &history, "fp");
        assert!(v.ratio.is_none());
        assert!(v.flags.iter().any(|f| f.contains("first appearance")));
    }

    #[test]
    fn before_the_chart_needs_all_evidence() {
        assert_eq!(before_the_chart(true, Some(0.5), Some(1.0)), Some(true));
        assert_eq!(before_the_chart(true, Some(8.0), Some(1.0)), Some(false));
        assert_eq!(before_the_chart(true, Some(0.5), Some(4.0)), Some(false));
        assert_eq!(before_the_chart(false, Some(0.5), Some(1.0)), Some(false));
        assert_eq!(before_the_chart(true, None, Some(1.0)), None);
    }
}
