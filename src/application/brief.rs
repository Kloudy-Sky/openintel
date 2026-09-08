//! IO edge for `brief`: the day's dated evidence with no ticker required —
//! market clock, vendored macro releases, the earnings calendar, overnight
//! filings and headlines for the names the user passes, and the last chatter
//! baseline. Each leg degrades into `errors` or `notes`; nothing is a silent
//! empty. Evidence only — no ranking, no picks.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::America::New_York;
use futures::StreamExt;
use serde::Serialize;

use crate::domain::brief::{headline_window, split_earnings, EarningsToday};
use crate::domain::chatter::BaselineLine;
use crate::domain::clock::{market_clock, MarketClock};
use crate::domain::dip::{filing_gate, headline_gate, GateEvidence, GateStatus};
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::macro_calendar;
use crate::domain::ports::earnings_calendar_source::EarningsCalendarSource;
use crate::domain::ports::filings_source::FilingsSource;
use crate::domain::ports::news_source::NewsSource;
use crate::domain::values::macro_release::MacroRelease;

const CONCURRENCY: usize = 4;
const HEADLINE_COUNT: usize = 20;
const TOP_MENTIONS: usize = 5;
const REGULAR_CLOSE: NaiveTime = NaiveTime::from_hms_opt(16, 0, 0).unwrap();
pub const DEFAULT_FLOOR_MARKET_CAP_USD: f64 = 500_000_000.0;

pub const FRAMING: &str = "brief lists dated evidence for the session ahead — it never ranks, \
     never predicts, and never recommends.";

pub struct BriefDeps<'a> {
    pub earnings: &'a dyn EarningsCalendarSource,
    pub news: &'a dyn NewsSource,
    pub filings: &'a dyn FilingsSource,
}

pub struct BriefRequest {
    /// Held and watched names: overnight filings and headlines are fetched for these.
    pub tickers: Vec<Ticker>,
    /// Earnings rows under this cap are counted, not listed (watched names always list).
    pub floor_market_cap_usd: f64,
    /// Chatter baseline journal to summarize; None skips the leg with a note.
    pub baseline_path: Option<PathBuf>,
}

impl Default for BriefRequest {
    fn default() -> Self {
        Self {
            tickers: Vec::new(),
            floor_market_cap_usd: DEFAULT_FLOOR_MARKET_CAP_USD,
            baseline_path: crate::application::chatter::default_baseline_path(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct HeadlineOut {
    pub title: String,
    pub publisher: String,
    pub published_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct TickerEvidence {
    pub ticker: String,
    /// Catalyst-form filings since the prior close, via the dip gate.
    pub filings: GateStatus,
    /// Company-referencing catalyst headlines, via the dip gate.
    pub headlines: GateStatus,
    pub evidence: Vec<String>,
    pub headlines_since_close: Vec<HeadlineOut>,
    /// Headlines the provider left undated: possibly recent, never dropped silently.
    pub undated_headlines: usize,
}

#[derive(Debug, Serialize)]
pub struct Mention {
    pub ticker: String,
    pub mentions: usize,
}

/// The most recent chatter baseline line per platform: counts only, the
/// velocity claim needs a fresh `discover --chatter` run.
#[derive(Debug, Serialize)]
pub struct ChatterSnapshot {
    pub date: NaiveDate,
    pub platform: String,
    pub total_posts: usize,
    pub top_mentions: Vec<Mention>,
}

#[derive(Debug, Serialize)]
pub struct BriefReport {
    pub generated_at: DateTime<Utc>,
    pub clock: MarketClock,
    /// "Since the prior close": the instant overnight evidence is measured from.
    pub since: DateTime<Utc>,
    pub macro_releases: Vec<MacroRelease>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub earnings: Option<EarningsToday>,
    pub tickers: Vec<TickerEvidence>,
    pub chatter: Vec<ChatterSnapshot>,
    pub errors: Vec<String>,
    pub notes: Vec<String>,
}

fn prior_close_instant(date: NaiveDate) -> Option<DateTime<Utc>> {
    New_York
        .from_local_datetime(&date.and_time(REGULAR_CLOSE))
        .single()
        .map(|t| t.with_timezone(&Utc))
}

async fn ticker_evidence(
    ticker: &Ticker,
    since_date: NaiveDate,
    since: DateTime<Utc>,
    deps: &BriefDeps<'_>,
) -> TickerEvidence {
    let filings = match deps.filings.recent_filings(ticker, since_date).await {
        Ok(f) => GateEvidence::Available(f),
        Err(e) => GateEvidence::Unavailable(e.to_string()),
    };
    let (headlines, company_names) = match deps.news.headlines(ticker, HEADLINE_COUNT).await {
        Ok(fetch) => (
            GateEvidence::Available(fetch.headlines),
            fetch.company_names,
        ),
        Err(e) => (GateEvidence::Unavailable(e.to_string()), Vec::new()),
    };
    let (filing_status, mut evidence) = filing_gate(&filings, since_date);
    let (headline_status, headline_evidence) =
        headline_gate(&headlines, ticker.as_str(), &company_names);
    evidence.extend(headline_evidence);

    let (recent, undated) = match &headlines {
        GateEvidence::Available(h) => headline_window(h, since),
        GateEvidence::Unavailable(_) => (Vec::new(), 0),
    };
    TickerEvidence {
        ticker: ticker.as_str().to_string(),
        filings: filing_status,
        headlines: headline_status,
        evidence,
        headlines_since_close: recent
            .into_iter()
            .filter_map(|h| {
                h.published_at.map(|at| HeadlineOut {
                    title: h.title,
                    publisher: h.publisher,
                    published_at: at,
                })
            })
            .collect(),
        undated_headlines: undated,
    }
}

/// Baseline lines plus how many lines failed to parse. A missing file is
/// Ok(empty): a journal that hasn't started yet. Any other read failure is
/// the caller's to report, never a quiet "no baseline".
fn read_baseline(path: &Path) -> std::io::Result<(Vec<BaselineLine>, usize)> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((Vec::new(), 0)),
        Err(e) => return Err(e),
    };
    let mut lines = Vec::new();
    let mut malformed = 0;
    for line in content.lines().filter(|l| !l.trim().is_empty()) {
        match serde_json::from_str(line) {
            Ok(parsed) => lines.push(parsed),
            Err(_) => malformed += 1,
        }
    }
    Ok((lines, malformed))
}

fn latest_baselines(lines: Vec<BaselineLine>) -> Vec<ChatterSnapshot> {
    let mut latest: BTreeMap<String, BaselineLine> = BTreeMap::new();
    for line in lines {
        let newer = latest
            .get(&line.platform)
            .is_none_or(|current| line.date > current.date);
        if newer {
            latest.insert(line.platform.clone(), line);
        }
    }
    latest
        .into_values()
        .map(|line| {
            let mut mentions: Vec<Mention> = line
                .counts
                .into_iter()
                .map(|(ticker, mentions)| Mention { ticker, mentions })
                .collect();
            mentions.sort_by(|a, b| b.mentions.cmp(&a.mentions).then(a.ticker.cmp(&b.ticker)));
            mentions.truncate(TOP_MENTIONS);
            ChatterSnapshot {
                date: line.date,
                platform: line.platform,
                total_posts: line.total_posts,
                top_mentions: mentions,
            }
        })
        .collect()
}

pub async fn brief(
    req: &BriefRequest,
    deps: &BriefDeps<'_>,
    now: DateTime<Utc>,
) -> Result<BriefReport, DomainError> {
    let mut errors = Vec::new();
    let mut notes = Vec::new();

    let clock = market_clock(now);
    let today = clock.date_et;
    if let Some(note) = &clock.note {
        notes.push(note.clone());
    }
    let since_date = match clock.prior_close_date {
        Some(d) => d,
        None => {
            let fallback = today.pred_opt().unwrap_or(today);
            notes.push(format!(
                "prior session date unverifiable; measuring overnight evidence from {fallback}"
            ));
            fallback
        }
    };
    let since = prior_close_instant(since_date).unwrap_or(now - chrono::Duration::hours(24));

    let macro_releases = match macro_calendar::releases_on(today) {
        Some(releases) => releases,
        None => {
            let (from, to) = macro_calendar::coverage();
            notes.push(format!(
                "macro calendar covers {from} to {to}; {today} is outside it, so scheduled releases are unknown"
            ));
            Vec::new()
        }
    };
    notes.push(macro_calendar::coverage_note().to_string());

    let watch: Vec<String> = req.tickers.iter().map(|t| t.as_str().to_string()).collect();
    let earnings = match deps.earnings.on(today).await {
        Ok(rows) => {
            if rows.is_empty() {
                notes.push(format!("earnings calendar lists nothing for {today}"));
            }
            Some(split_earnings(rows, &watch, req.floor_market_cap_usd))
        }
        Err(e) => {
            errors.push(format!("earnings calendar unavailable: {e}"));
            None
        }
    };

    let mut indexed: Vec<(usize, TickerEvidence)> = futures::stream::iter(
        req.tickers
            .iter()
            .cloned()
            .enumerate()
            .map(|(i, t)| async move { (i, ticker_evidence(&t, since_date, since, deps).await) }),
    )
    .buffer_unordered(CONCURRENCY)
    .collect()
    .await;
    indexed.sort_by_key(|(i, _)| *i);
    let tickers: Vec<TickerEvidence> = indexed.into_iter().map(|(_, t)| t).collect();

    let chatter = match &req.baseline_path {
        None => {
            notes.push("chatter baseline path not set; chatter leg skipped".into());
            Vec::new()
        }
        Some(path) => match read_baseline(path) {
            Err(e) => {
                errors.push(format!(
                    "chatter baseline unreadable at {}: {e}",
                    path.display()
                ));
                Vec::new()
            }
            Ok((lines, malformed)) => {
                if malformed > 0 {
                    notes.push(format!(
                        "{malformed} malformed line(s) skipped in the chatter baseline"
                    ));
                }
                let snapshots = latest_baselines(lines);
                if snapshots.is_empty() {
                    notes.push(
                        "no chatter baseline yet; run `discover --chatter` after the close".into(),
                    );
                } else if let Some(oldest) = snapshots.iter().map(|s| s.date).min() {
                    if oldest < since_date {
                        notes.push(format!(
                            "chatter baseline last written {oldest}, before the prior close; counts are stale"
                        ));
                    }
                }
                snapshots
            }
        },
    };

    Ok(BriefReport {
        generated_at: now,
        clock,
        since,
        macro_releases,
        earnings,
        tickers,
        chatter,
        errors,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::calendar::mock_calendar::MockEarningsCalendar;
    use crate::adapters::filings::mock_filings::MockFilingsSource;
    use crate::adapters::market::mock_news::MockNewsSource;
    use crate::domain::ports::news_source::NewsFetch;
    use crate::domain::values::earnings_row::{EarningsRow, ReportSlot};
    use crate::domain::values::headline::Headline;

    fn tuesday_pre_market() -> DateTime<Utc> {
        New_York
            .with_ymd_and_hms(2026, 9, 8, 8, 30, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc)
    }

    fn req(tickers: &[&str], baseline: Option<PathBuf>) -> BriefRequest {
        BriefRequest {
            tickers: tickers.iter().map(|t| Ticker::parse(t).unwrap()).collect(),
            floor_market_cap_usd: DEFAULT_FLOOR_MARKET_CAP_USD,
            baseline_path: baseline,
        }
    }

    #[tokio::test]
    async fn every_failing_leg_is_an_error_or_a_note_never_a_silent_empty() {
        let earnings = MockEarningsCalendar(Err("down".into()));
        let news = MockNewsSource(Err("no news".into()));
        let filings = MockFilingsSource(Err("edgar down".into()));
        let deps = BriefDeps {
            earnings: &earnings,
            news: &news,
            filings: &filings,
        };
        let report = brief(&req(&["ADSK"], None), &deps, tuesday_pre_market())
            .await
            .unwrap();

        let unreadable = brief(
            &req(&["ADSK"], Some(std::env::temp_dir())),
            &deps,
            tuesday_pre_market(),
        )
        .await
        .unwrap();
        assert!(unreadable
            .errors
            .iter()
            .any(|e| e.contains("chatter baseline unreadable")));
        assert!(!unreadable
            .notes
            .iter()
            .any(|n| n.contains("no chatter baseline yet")));

        assert_eq!(
            report.clock.prior_close_date,
            Some(NaiveDate::from_ymd_opt(2026, 9, 4).unwrap())
        );
        assert!(report.earnings.is_none());
        assert!(report
            .errors
            .iter()
            .any(|e| e.contains("earnings calendar unavailable")));
        let t = &report.tickers[0];
        assert!(matches!(t.filings, GateStatus::Unknown(_)));
        assert!(matches!(t.headlines, GateStatus::Unknown(_)));
        assert!(t.headlines_since_close.is_empty());
        assert!(report.chatter.is_empty());
        assert!(report
            .notes
            .iter()
            .any(|n| n.contains("chatter leg skipped")));
    }

    #[tokio::test]
    async fn happy_path_buckets_earnings_windows_headlines_and_reads_the_baseline() {
        let earnings = MockEarningsCalendar(Ok(vec![
            EarningsRow {
                symbol: "CASY".into(),
                company: "Casey's".into(),
                slot: ReportSlot::AfterClose,
                market_cap_usd: Some(28e9),
                eps_forecast: Some(6.6),
                fiscal_quarter: None,
            },
            EarningsRow {
                symbol: "TINY".into(),
                company: "Tiny".into(),
                slot: ReportSlot::BeforeOpen,
                market_cap_usd: Some(40e6),
                eps_forecast: None,
                fiscal_quarter: None,
            },
        ]));
        let friday_evening = New_York
            .with_ymd_and_hms(2026, 9, 4, 18, 0, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        let news = MockNewsSource(Ok(NewsFetch {
            headlines: vec![
                Headline {
                    title: "Autodesk names new CFO".into(),
                    publisher: "wire".into(),
                    published_at: Some(friday_evening),
                },
                Headline {
                    title: "old story".into(),
                    publisher: "wire".into(),
                    published_at: Some(friday_evening - chrono::Duration::days(3)),
                },
            ],
            company_names: vec!["Autodesk".into()],
        }));
        let filings = MockFilingsSource(Ok(Vec::new()));
        let deps = BriefDeps {
            earnings: &earnings,
            news: &news,
            filings: &filings,
        };

        let dir = std::env::temp_dir().join(format!("openintel-brief-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("baseline.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"date":"2026-09-03","platform":"bluesky","fingerprint":"f","total_posts":10,"counts":{"TSLA":1}}"#,
                "\n",
                r#"{"date":"2026-09-04","platform":"bluesky","fingerprint":"f","total_posts":13,"counts":{"TSLA":3,"NVDA":2,"AAPL":2}}"#,
                "\n"
            ),
        )
        .unwrap();

        let report = brief(&req(&["ADSK"], Some(path)), &deps, tuesday_pre_market())
            .await
            .unwrap();

        let e = report.earnings.unwrap();
        assert_eq!(e.after_close[0].symbol, "CASY");
        assert!(e.before_open.is_empty());
        assert_eq!(e.below_floor, 1);

        let t = &report.tickers[0];
        assert_eq!(t.filings, GateStatus::Pass);
        assert_eq!(t.headlines_since_close.len(), 1);
        assert_eq!(t.headlines_since_close[0].title, "Autodesk names new CFO");

        assert_eq!(report.chatter.len(), 1);
        let c = &report.chatter[0];
        assert_eq!(c.date, NaiveDate::from_ymd_opt(2026, 9, 4).unwrap());
        assert_eq!(c.top_mentions[0].ticker, "TSLA");
        assert_eq!(c.top_mentions[1].ticker, "AAPL");
        assert!(!report.notes.iter().any(|n| n.contains("counts are stale")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
