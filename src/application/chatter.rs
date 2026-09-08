//! IO edge for `discover --chatter`: fetch the listening set's recent posts
//! per platform, count cashtag mentions, compute velocity against the
//! baseline journal, append today's baseline line, and annotate surfaced
//! tickers with the tape (change, RVOL) for the before-the-chart read.
//! Attention is the signal being measured — the output says so and never
//! ranks or recommends.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::config::listening::ListeningSet;
use crate::domain::chatter::{
    before_the_chart, count_mentions, velocity, BaselineLine, MentionCounts, Velocity,
    MIN_REPORT_MENTIONS,
};
use crate::domain::entities::pulse::PulsePost;
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::ports::listening_feed::ListeningFeed;
use crate::domain::ports::market_data_source::MarketDataSource;

pub const FRAMING: &str = "chatter measures ATTENTION, not information — one platform's \
     voices, easily brigaded. It never ranks, never predicts, never recommends. \
     Cross-check anything interesting with analyze_ticker and the catalyst gates.";

/// Default paid-read ceiling for the X leg — ~$0.10 at $0.005/read.
pub const DEFAULT_X_READ_CAP: usize = 20;
pub const DEFAULT_FREE_LIMIT: usize = 100;
pub const DEFAULT_HOURS: u32 = 24;
/// Recent posts included per platform for the agent to read — names beat
/// cashtags in influencer language, and the reader connects them.
const TOP_POSTS: usize = 10;

fn fail(message: impl Into<String>) -> DomainError {
    DomainError::SourceFailure {
        name: "chatter".into(),
        message: message.into(),
    }
}

/// `~/.openintel/chatter_baseline.jsonl`, or None when no home dir resolves.
pub fn default_baseline_path() -> Option<PathBuf> {
    std::env::home_dir().map(|h| h.join(".openintel").join("chatter_baseline.jsonl"))
}

#[derive(Clone)]
pub struct ChatterRequest {
    pub listening: ListeningSet,
    pub hours: u32,
    /// Post cap for free platforms.
    pub free_limit: usize,
    /// Paid-read ceiling for X — every returned post bills ~$0.005.
    pub x_read_cap: usize,
    /// None = no baseline read or write (velocity reports "baseline building").
    pub baseline_path: Option<PathBuf>,
    /// Append today's line after reading. A read-only pass (the watch loop)
    /// sets this false so repeated runs can't stack a day's worth of lines.
    pub write_baseline: bool,
}

#[derive(Debug, Serialize)]
pub struct TickerChatter {
    pub ticker: String,
    pub velocity: Velocity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change_pct: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rvol: Option<f64>,
    /// Elevated chatter while the tape is quiet. None = tape unavailable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_the_chart: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct PlatformChatter {
    pub platform: String,
    pub handles: Vec<String>,
    pub posts_scanned: usize,
    pub posts_returned: u32,
    /// Real money spent on this leg (paid platforms only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_cost_usd: Option<f64>,
    pub tickers: Vec<TickerChatter>,
    pub recent_posts: Vec<PulsePost>,
    pub notes: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ChatterReport {
    pub generated_at: DateTime<Utc>,
    pub hours: u32,
    pub listening_fingerprint: String,
    pub platforms: Vec<PlatformChatter>,
    pub errors: Vec<String>,
    pub notes: Vec<String>,
}

fn read_baseline(path: &Path) -> Vec<BaselineLine> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

fn append_baseline(path: &Path, line: &BaselineLine) -> Result<(), DomainError> {
    use std::io::Write as _;
    let json = serde_json::to_string(line).map_err(|e| fail(e.to_string()))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| fail(e.to_string()))?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| fail(e.to_string()))?;
    writeln!(file, "{json}").map_err(|e| fail(e.to_string()))
}

async fn tape_for(
    ticker: &str,
    market: Option<&dyn MarketDataSource>,
) -> (Option<f64>, Option<f64>) {
    let Some(market) = market else {
        return (None, None);
    };
    let Ok(parsed) = Ticker::parse(ticker) else {
        return (None, None);
    };
    match market.snapshot(&parsed).await {
        Ok(snap) => {
            let change = (snap.previous_close > 0.0)
                .then(|| (snap.last_price - snap.previous_close) / snap.previous_close * 100.0);
            let rvol = (snap.avg_volume > 0).then(|| snap.volume as f64 / snap.avg_volume as f64);
            (change, rvol)
        }
        Err(_) => (None, None),
    }
}

/// One platform leg: fetch, count, grade velocity, journal, surface.
async fn platform_leg(
    feed: &dyn ListeningFeed,
    req: &ChatterRequest,
    fingerprint: &str,
    market: Option<&dyn MarketDataSource>,
    now: DateTime<Utc>,
) -> Result<PlatformChatter, DomainError> {
    let platform = feed.platform().to_string();
    let handles = req.listening.handles_for(&platform).to_vec();
    let limit = if feed.paid() {
        req.x_read_cap.min(100)
    } else {
        req.free_limit.min(100)
    };
    let mut notes = Vec::new();

    let fetch = feed.listening_posts(&handles, req.hours, limit).await?;
    let texts: Vec<&str> = fetch.posts.iter().map(|p| p.text.as_str()).collect();
    let counts: MentionCounts = count_mentions(&texts);

    let today = now
        .with_timezone(&chrono_tz::America::New_York)
        .date_naive();
    let history: Vec<BaselineLine> = match &req.baseline_path {
        Some(path) => read_baseline(path)
            .into_iter()
            .filter(|l| l.platform == platform && l.date < today)
            .collect(),
        None => Vec::new(),
    };

    let mut tickers = Vec::new();
    for (sym, n) in &counts.counts {
        if *n < MIN_REPORT_MENTIONS {
            continue;
        }
        let v = velocity(sym, &counts, &history, fingerprint);
        let (change_pct, rvol) = tape_for(sym, market).await;
        let btc = before_the_chart(v.elevated, change_pct, rvol);
        tickers.push(TickerChatter {
            ticker: sym.clone(),
            velocity: v,
            change_pct,
            rvol,
            before_the_chart: btc,
        });
    }
    if tickers.is_empty() && counts.total_posts > 0 {
        notes.push(format!(
            "no ticker reached {MIN_REPORT_MENTIONS} cashtag mentions in {} posts — read the recent posts instead; influencer language names companies, not cashtags",
            counts.total_posts
        ));
    }

    if let Some(path) = req.baseline_path.as_ref().filter(|_| req.write_baseline) {
        let line = BaselineLine {
            date: today,
            platform: platform.clone(),
            fingerprint: fingerprint.to_string(),
            total_posts: counts.total_posts,
            counts: counts.counts.clone(),
        };
        if let Err(e) = append_baseline(path, &line) {
            notes.push(format!("baseline write failed: {e}"));
        }
    }

    let mut recent = fetch.posts.clone();
    recent.sort_by_key(|p| std::cmp::Reverse(p.created_at));
    recent.truncate(TOP_POSTS);

    Ok(PlatformChatter {
        estimated_cost_usd: feed.paid().then(|| {
            f64::from(fetch.posts_returned) * crate::application::pulse::X_COST_PER_READ_USD
        }),
        platform,
        handles,
        posts_scanned: fetch.posts.len(),
        posts_returned: fetch.posts_returned,
        tickers,
        recent_posts: recent,
        notes,
    })
}

pub async fn chatter(
    req: &ChatterRequest,
    feeds: &[&dyn ListeningFeed],
    market: Option<&dyn MarketDataSource>,
    now: DateTime<Utc>,
) -> Result<ChatterReport, DomainError> {
    if feeds.is_empty() {
        return Err(fail(
            "no listening feeds available — configure bluesky/reddit (openintel setup) or opt in to x",
        ));
    }
    let fingerprint = req.listening.fingerprint();
    let mut platforms = Vec::new();
    let mut errors = Vec::new();
    // Sequential: at most 3 legs, and the paid leg must never race a failure.
    for feed in feeds {
        match platform_leg(*feed, req, &fingerprint, market, now).await {
            Ok(p) => platforms.push(p),
            Err(e) => errors.push(format!("{}: {e}", feed.platform())),
        }
    }

    let mut notes = Vec::new();
    if platforms
        .iter()
        .all(|p| p.tickers.iter().all(|t| t.velocity.ratio.is_none()))
    {
        notes.push(
            "no velocity claims yet — the baseline matures with daily runs; raw counts and posts only"
                .into(),
        );
    }

    Ok(ChatterReport {
        generated_at: now,
        hours: req.hours,
        listening_fingerprint: fingerprint,
        platforms,
        errors,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entities::social_post::PostText;
    use async_trait::async_trait;
    use chrono::TimeZone;

    struct FixedFeed {
        platform: &'static str,
        paid: bool,
        texts: Vec<&'static str>,
    }

    #[async_trait]
    impl ListeningFeed for FixedFeed {
        fn platform(&self) -> &'static str {
            self.platform
        }
        fn paid(&self) -> bool {
            self.paid
        }
        async fn listening_posts(
            &self,
            _handles: &[String],
            _hours: u32,
            _limit: usize,
        ) -> Result<crate::domain::ports::listening_feed::ListeningFetch, DomainError> {
            let posts = self
                .texts
                .iter()
                .enumerate()
                .map(|(i, t)| PulsePost {
                    id: format!("{}-{i}", self.platform),
                    author: "someone".into(),
                    text: PostText::parse(t).unwrap(),
                    created_at: Utc.with_ymd_and_hms(2026, 8, 21, 12, 0, 0).unwrap(),
                    engagement: 1,
                })
                .collect::<Vec<_>>();
            let returned = posts.len() as u32;
            Ok(crate::domain::ports::listening_feed::ListeningFetch {
                posts,
                posts_returned: returned,
            })
        }
    }

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 21, 21, 0, 0).unwrap()
    }

    fn req(baseline: Option<PathBuf>) -> ChatterRequest {
        ChatterRequest {
            listening: ListeningSet::default(),
            hours: 24,
            free_limit: 100,
            x_read_cap: 20,
            baseline_path: baseline,
            write_baseline: true,
        }
    }

    #[tokio::test]
    async fn cold_start_reports_counts_without_velocity_and_journals_baseline() {
        let dir = std::env::temp_dir().join(format!("openintel-chatter-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("baseline.jsonl");

        let feed = FixedFeed {
            platform: "bluesky",
            paid: false,
            texts: vec!["$NVDA one", "$NVDA two", "$NVDA three", "nothing"],
        };
        let report = chatter(&req(Some(path.clone())), &[&feed], None, now())
            .await
            .unwrap();

        let p = &report.platforms[0];
        assert_eq!(p.tickers.len(), 1);
        assert_eq!(p.tickers[0].velocity.mentions, 3);
        assert!(p.tickers[0].velocity.ratio.is_none());
        assert!(p.estimated_cost_usd.is_none());
        assert!(report.notes.iter().any(|n| n.contains("baseline matures")));

        let line: BaselineLine = serde_json::from_str(
            std::fs::read_to_string(&path)
                .unwrap()
                .lines()
                .next()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(line.platform, "bluesky");
        assert_eq!(line.counts["NVDA"], 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn paid_leg_reports_cost_and_no_cashtags_notes_the_language_gap() {
        let feed = FixedFeed {
            platform: "x",
            paid: true,
            texts: vec!["Tesla robotaxi approval coming", "Tariffs on chips"],
        };
        let report = chatter(&req(None), &[&feed], None, now()).await.unwrap();
        let p = &report.platforms[0];
        assert_eq!(p.estimated_cost_usd, Some(2.0 * 0.005));
        assert!(p.tickers.is_empty());
        assert!(p.notes.iter().any(|n| n.contains("influencer language")));
        assert_eq!(p.recent_posts.len(), 2);
    }

    #[tokio::test]
    async fn failed_leg_is_an_error_not_a_sunk_run() {
        struct Broken;
        #[async_trait]
        impl ListeningFeed for Broken {
            fn platform(&self) -> &'static str {
                "reddit"
            }
            fn paid(&self) -> bool {
                false
            }
            async fn listening_posts(
                &self,
                _h: &[String],
                _hours: u32,
                _l: usize,
            ) -> Result<crate::domain::ports::listening_feed::ListeningFetch, DomainError>
            {
                Err(fail("boom"))
            }
        }
        let ok = FixedFeed {
            platform: "bluesky",
            paid: false,
            texts: vec!["$NVDA a", "$NVDA b", "$NVDA c"],
        };
        let report = chatter(&req(None), &[&Broken, &ok], None, now())
            .await
            .unwrap();
        assert_eq!(report.platforms.len(), 1);
        assert!(report.errors.iter().any(|e| e.contains("reddit")));
    }

    #[tokio::test]
    async fn no_feeds_is_a_clean_error() {
        assert!(chatter(&req(None), &[], None, now()).await.is_err());
    }
}
