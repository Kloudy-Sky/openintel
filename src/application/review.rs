//! dip --review orchestration: read the scan journal, fetch bar history for
//! every journaled ticker plus the index proxy, grade forward returns, and
//! aggregate by verdict. This is how the v0 dip weights earn (or lose) trust.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use futures::StreamExt;
use serde::{Deserialize, Serialize};

use crate::application::dip::{EntryError, SPX_PROXY};
use crate::domain::dip::{Session, Verdict};
use crate::domain::dip_review::{
    aggregate, forward_returns, pearson, ForwardReturns, GradedEntry, VerdictBucket,
};
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::ports::bar_source::BarSource;
use crate::domain::values::bar::Bar;

const CONCURRENCY: usize = 5;
/// Below this graded sample size the report carries a not-meaningful note.
const MEANINGFUL_N: usize = 30;

pub const REVIEW_FRAMING: &str = "dip --review grades past scans against subsequent prices — \
past setup conformance is not evidence of future returns. Small samples prove nothing.";

fn fail(message: impl Into<String>) -> DomainError {
    DomainError::SourceFailure {
        name: "dip-review".into(),
        message: message.into(),
    }
}

// Owned mirror of the journal line the scan writes (application::dip).
#[derive(Debug, Deserialize)]
struct JournalLineIn {
    generated_at: DateTime<Utc>,
    session: Session,
    candidates: Vec<JournalCandidateIn>,
}

#[derive(Debug, Deserialize)]
struct JournalCandidateIn {
    ticker: String,
    price: f64,
    score: f64,
    verdict: Verdict,
}

#[derive(Debug, Serialize)]
pub struct DipReviewReport {
    pub generated_at: DateTime<Utc>,
    pub journal_path: String,
    pub scans: usize,
    pub entries_total: usize,
    /// Entries with at least a 1-day forward return.
    pub graded: usize,
    /// Too recent — the market hasn't produced enough forward bars yet.
    pub pending: usize,
    /// Older than the available bar history (~3 months) — ungradable.
    pub stale: usize,
    pub deduped: usize,
    pub skipped_lines: usize,
    pub buckets: Vec<VerdictBucket>,
    /// Pearson of composite score vs raw 5-day return across graded entries.
    pub score_return_corr_d5: Option<f64>,
    pub entries: Vec<GradedEntry>,
    pub errors: Vec<EntryError>,
    pub notes: Vec<String>,
}

struct PendingEntry {
    ticker: String,
    scanned_on: chrono::NaiveDate,
    session: Session,
    verdict: Verdict,
    score: f64,
    price: f64,
}

fn parse_journal(content: &str) -> (Vec<PendingEntry>, usize, usize, usize) {
    let mut entries = Vec::new();
    let mut scans = 0usize;
    let mut skipped = 0usize;
    let mut deduped = 0usize;
    let mut seen: HashSet<(String, chrono::NaiveDate)> = HashSet::new();
    for line in content.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(parsed) = serde_json::from_str::<JournalLineIn>(line) else {
            skipped += 1;
            continue;
        };
        scans += 1;
        let scanned_on = parsed
            .generated_at
            .with_timezone(&chrono_tz::America::New_York)
            .date_naive();
        for c in parsed.candidates {
            if !seen.insert((c.ticker.clone(), scanned_on)) {
                deduped += 1;
                continue;
            }
            entries.push(PendingEntry {
                ticker: c.ticker,
                scanned_on,
                session: parsed.session,
                verdict: c.verdict,
                score: c.score,
                price: c.price,
            });
        }
    }
    (entries, scans, skipped, deduped)
}

/// SPY's forward returns from its own close on the scan date — the baseline
/// the entry's returns are adjusted against.
fn index_forward(spy_bars: &[Bar], scanned_on: chrono::NaiveDate) -> ForwardReturns {
    match spy_bars.iter().find(|b| b.date == scanned_on) {
        Some(base) => forward_returns(scanned_on, base.close, spy_bars),
        None => ForwardReturns::default(),
    }
}

fn minus(raw: ForwardReturns, base: ForwardReturns) -> ForwardReturns {
    let sub = |a: Option<f64>, b: Option<f64>| match (a, b) {
        (Some(a), Some(b)) => Some(a - b),
        _ => None,
    };
    ForwardReturns {
        d1: sub(raw.d1, base.d1),
        d5: sub(raw.d5, base.d5),
        d10: sub(raw.d10, base.d10),
    }
}

pub async fn dip_review(
    journal_path: &Path,
    bars_src: &dyn BarSource,
    now: DateTime<Utc>,
) -> Result<DipReviewReport, DomainError> {
    let content = std::fs::read_to_string(journal_path).map_err(|e| {
        fail(format!(
            "no journal at {} ({e}) — run some scans first",
            journal_path.display()
        ))
    })?;
    let (pending_entries, scans, skipped_lines, deduped) = parse_journal(&content);
    if pending_entries.is_empty() {
        return Err(fail("journal has no gradable candidate entries"));
    }

    let mut tickers: Vec<String> = pending_entries
        .iter()
        .map(|e| e.ticker.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    tickers.sort();
    tickers.push(SPX_PROXY.to_string());

    let fetched: Vec<(String, Result<Vec<Bar>, DomainError>)> =
        futures::stream::iter(tickers.into_iter().map(|symbol| async move {
            let result = match Ticker::parse(&symbol) {
                Ok(t) => bars_src.bars(&t).await,
                Err(e) => Err(e),
            };
            (symbol, result)
        }))
        .buffer_unordered(CONCURRENCY)
        .collect()
        .await;

    let mut history: HashMap<String, Vec<Bar>> = HashMap::new();
    let mut errors: Vec<EntryError> = Vec::new();
    for (symbol, result) in fetched {
        match result {
            Ok(bars) => {
                history.insert(symbol, bars);
            }
            Err(e) => errors.push(EntryError {
                ticker: symbol,
                error: e.to_string(),
            }),
        }
    }
    let spy_bars = history.get(SPX_PROXY).cloned().unwrap_or_default();

    let mut entries: Vec<GradedEntry> = Vec::new();
    let (mut graded, mut pending, mut stale) = (0usize, 0usize, 0usize);
    let entries_total = pending_entries.len();
    for e in pending_entries {
        let Some(bars) = history.get(&e.ticker) else {
            continue; // fetch error already recorded
        };
        let returns = forward_returns(e.scanned_on, e.price, bars);
        if returns.d1.is_some() {
            graded += 1;
        } else if bars.first().is_some_and(|b| e.scanned_on < b.date) {
            stale += 1;
            continue;
        } else {
            pending += 1;
            continue;
        }
        let excess = minus(returns, index_forward(&spy_bars, e.scanned_on));
        entries.push(GradedEntry {
            ticker: e.ticker,
            scanned_on: e.scanned_on,
            session: e.session,
            verdict: e.verdict,
            score: e.score,
            entry_price: e.price,
            returns,
            excess,
        });
    }

    let buckets = aggregate(&entries);
    let pairs: Vec<(f64, f64)> = entries
        .iter()
        .filter_map(|e| e.returns.d5.map(|r| (e.score, r)))
        .collect();
    let xs: Vec<f64> = pairs.iter().map(|p| p.0).collect();
    let ys: Vec<f64> = pairs.iter().map(|p| p.1).collect();
    let score_return_corr_d5 = pearson(&xs, &ys);

    let mut notes: Vec<String> = Vec::new();
    if graded < MEANINGFUL_N {
        notes.push(format!(
            "graded sample n={graded} < {MEANINGFUL_N} — NOT statistically meaningful; keep journaling"
        ));
    }
    if stale > 0 {
        notes.push(format!(
            "{stale} entr{} older than available bar history (~3 months) — ungradable",
            if stale == 1 { "y is" } else { "ies are" }
        ));
    }
    if skipped_lines > 0 {
        notes.push(format!("{skipped_lines} malformed journal line(s) skipped"));
    }
    if spy_bars.is_empty() {
        notes.push(format!(
            "index proxy {SPX_PROXY} history unavailable — excess returns not computed"
        ));
    }

    Ok(DipReviewReport {
        generated_at: now,
        journal_path: journal_path.display().to_string(),
        scans,
        entries_total,
        graded,
        pending,
        stale,
        deduped,
        skipped_lines,
        buckets,
        score_return_corr_d5,
        entries,
        errors,
        notes,
    })
}

/// Convenience for callers that grade the default journal location.
pub fn default_journal_or_err() -> Result<PathBuf, DomainError> {
    crate::application::dip::default_journal_path()
        .ok_or_else(|| fail("cannot resolve a home directory for the journal path"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use chrono::NaiveDate;

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

    struct MapBars(HashMap<String, Vec<Bar>>);

    #[async_trait]
    impl BarSource for MapBars {
        async fn bars(&self, t: &Ticker) -> Result<Vec<Bar>, DomainError> {
            self.0.get(t.as_str()).cloned().ok_or(DomainError::NoData)
        }
    }

    /// Two scans: one on the 10th (gradable), a duplicate GOOD entry on the
    /// 10th (deduped), and one very recent scan (pending).
    fn journal() -> String {
        [
            r#"{"generated_at":"2026-08-10T21:30:00Z","session":"post_close","spx_change_pct":-0.4,"candidates":[
                {"ticker":"GOOD","change_pct":-8.0,"price":100.0,"score":70.0,"verdict":"high_confidence"},
                {"ticker":"MEH","change_pct":-6.0,"price":50.0,"score":30.0,"verdict":"watch"}]}"#
                .replace('\n', ""),
            r#"{"generated_at":"2026-08-10T22:00:00Z","session":"post_close","spx_change_pct":-0.4,"candidates":[
                {"ticker":"GOOD","change_pct":-8.0,"price":100.0,"score":70.0,"verdict":"high_confidence"}]}"#
                .replace('\n', ""),
            r#"{"generated_at":"2026-08-20T21:30:00Z","session":"post_close","spx_change_pct":0.1,"candidates":[
                {"ticker":"GOOD","change_pct":-5.0,"price":90.0,"score":40.0,"verdict":"watch"}]}"#
                .replace('\n', ""),
            "not json at all".to_string(),
        ]
        .join("\n")
    }

    fn history() -> MapBars {
        let mut m = HashMap::new();
        // GOOD: bars through the 20th; scan on the 10th grades d1/d5, the
        // scan on the 20th has no forward bars yet (pending)
        m.insert(
            "GOOD".to_string(),
            (8..=20)
                .map(|i| bar(i, 100.0 + (i as f64 - 10.0)))
                .collect(),
        );
        // MEH drifts down
        m.insert(
            "MEH".to_string(),
            (8..=20)
                .map(|i| bar(i, 50.0 - (i as f64 - 10.0) * 0.5))
                .collect(),
        );
        // SPY flat
        m.insert(
            SPX_PROXY.to_string(),
            (8..=20).map(|i| bar(i, 500.0)).collect(),
        );
        MapBars(m)
    }

    #[tokio::test]
    async fn review_grades_dedupes_and_flags_pending() {
        let dir = std::env::temp_dir().join(format!("openintel-review-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("journal.jsonl");
        std::fs::write(&path, journal()).unwrap();

        let now = chrono::Utc::now();
        let report = dip_review(&path, &history(), now).await.unwrap();

        assert_eq!(report.scans, 3);
        assert_eq!(report.skipped_lines, 1);
        assert_eq!(report.deduped, 1);
        assert_eq!(report.entries_total, 3); // GOOD@10, MEH@10, GOOD@20
        assert_eq!(report.graded, 2);
        assert_eq!(report.pending, 1); // the scan on the 20th
        assert_eq!(report.stale, 0);

        // buckets ordered strongest first, math spot-checked:
        // GOOD@10 entry 100 -> d1 close 101 (+1%), d5 close 105 (+5%)
        let hc = &report.buckets[0];
        assert_eq!(hc.verdict, Verdict::HighConfidence);
        assert!((hc.raw.d1.as_ref().unwrap().mean_pct - 1.0).abs() < 1e-9);
        assert!((hc.raw.d5.as_ref().unwrap().mean_pct - 5.0).abs() < 1e-9);
        // SPY flat -> excess == raw
        assert!((hc.excess.d5.as_ref().unwrap().mean_pct - 5.0).abs() < 1e-9);
        // MEH: entry 50 -> d1 close 49.5 (-1%)
        let watch = &report.buckets[1];
        assert_eq!(watch.verdict, Verdict::Watch);
        assert!((watch.raw.d1.as_ref().unwrap().mean_pct + 1.0).abs() < 1e-9);

        // small-n honesty note always present at n=2
        assert!(report
            .notes
            .iter()
            .any(|n| n.contains("NOT statistically meaningful")));
        // corr needs >= 3 pairs -> None here
        assert!(report.score_return_corr_d5.is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn review_handles_missing_journal_and_fetch_errors() {
        let missing = std::env::temp_dir().join("openintel-no-such-journal.jsonl");
        let err = dip_review(&missing, &history(), chrono::Utc::now())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("run some scans first"));

        // a ticker with no bar history becomes a per-entry error, not a crash
        let dir = std::env::temp_dir().join(format!("openintel-review-err-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("journal.jsonl");
        std::fs::write(
            &path,
            r#"{"generated_at":"2026-08-10T21:30:00Z","session":"post_close","candidates":[{"ticker":"GONE","change_pct":-8.0,"price":10.0,"score":70.0,"verdict":"watch"}]}"#,
        )
        .unwrap();
        let report = dip_review(&path, &history(), chrono::Utc::now())
            .await
            .unwrap();
        assert_eq!(report.graded, 0);
        assert!(report.errors.iter().any(|e| e.ticker == "GONE"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
