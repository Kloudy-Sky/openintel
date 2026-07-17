use chrono::{DateTime, Utc};

use crate::domain::entities::pulse::PulseReport;
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::ports::influencer_feed::InfluencerFeed;

/// X username charset: letters, digits, underscore, max 15 chars.
fn is_valid_handle(a: &str) -> bool {
    !a.is_empty() && a.len() <= 15 && a.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// X pay-per-use price per post read (docs.x.com pricing, 2026-02 launch).
pub const X_COST_PER_READ_USD: f64 = 0.005;

/// No-arguments fallback: market-moving macro accounts. Per-call account
/// lists are the primary path — the consuming agent curates per ticker.
pub const DEFAULT_PULSE_ACCOUNTS: [&str; 4] = [
    "realDonaldTrump",
    "WhiteHouse",
    "elonmusk",
    "federalreserve",
];

/// X recent search covers 7 days; cap below the boundary so start_time never
/// lands outside the window mid-flight.
pub const MAX_HOURS_BACK: u32 = 167;
pub const MAX_PULSE_LIMIT: usize = 100;

/// Trim, strip a leading `@`, drop invalid handles (X username charset:
/// letters, digits, underscore, max 15 chars); empty raw input -> the default
/// list. If raw was non-empty but every handle was invalid, error rather than
/// silently falling back to defaults — that would spend money on accounts the
/// user didn't choose.
pub fn normalize_accounts(raw: &[String]) -> Result<Vec<String>, DomainError> {
    if raw.is_empty() {
        return Ok(DEFAULT_PULSE_ACCOUNTS
            .iter()
            .map(|s| s.to_string())
            .collect());
    }
    let cleaned: Vec<String> = raw
        .iter()
        .map(|a| a.trim().trim_start_matches('@').to_string())
        .filter(|a| is_valid_handle(a))
        .collect();
    if cleaned.is_empty() {
        return Err(DomainError::SourceFailure {
            name: "x".into(),
            message: format!(
                "no valid X handles in {raw:?} (letters, digits, underscore, max 15 chars)"
            ),
        });
    }
    Ok(cleaned)
}

pub async fn pulse(
    ticker_raw: &str,
    accounts_raw: &[String],
    hours_back: u32,
    limit: usize,
    feed: &dyn InfluencerFeed,
    now: DateTime<Utc>,
) -> Result<PulseReport, DomainError> {
    let ticker = Ticker::parse(ticker_raw)?;
    let accounts = normalize_accounts(accounts_raw)?;
    let hours_back = hours_back.clamp(1, MAX_HOURS_BACK);
    let limit = limit.clamp(1, MAX_PULSE_LIMIT);
    let fetch = feed.pulse(&ticker, &accounts, hours_back, limit).await?;
    let posts = fetch.posts;
    let posts_read = fetch.posts_returned;
    Ok(PulseReport {
        ticker: ticker.as_str().to_string(),
        accounts,
        hours_back,
        posts,
        posts_read,
        estimated_cost_usd: f64::from(posts_read) * X_COST_PER_READ_USD,
        generated_at: now,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entities::pulse::{PulseFetch, PulsePost};
    use crate::domain::entities::social_post::PostText;
    use async_trait::async_trait;
    use chrono::TimeZone;

    fn at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 16, 12, 0, 0).unwrap()
    }

    /// (ticker, accounts, hours_back, limit) the fake was called with.
    type SeenCall = (String, Vec<String>, u32, usize);

    /// Records what it was called with; returns `n` canned posts and
    /// `posts_returned` (defaults to `n` via `fake()`, overridable via
    /// `fake_with_returned()` to simulate billing > kept posts).
    struct FakeFeed {
        n: usize,
        posts_returned: u32,
        seen: std::sync::Mutex<Option<SeenCall>>,
    }

    #[async_trait]
    impl InfluencerFeed for FakeFeed {
        async fn pulse(
            &self,
            ticker: &Ticker,
            accounts: &[String],
            hours_back: u32,
            limit: usize,
        ) -> Result<PulseFetch, DomainError> {
            *self.seen.lock().unwrap() = Some((
                ticker.as_str().to_string(),
                accounts.to_vec(),
                hours_back,
                limit,
            ));
            Ok(PulseFetch {
                posts: (0..self.n)
                    .map(|i| PulsePost {
                        id: format!("p{i}"),
                        author: "someone".into(),
                        text: PostText::parse("hello market").unwrap(),
                        created_at: at(),
                        engagement: 1,
                    })
                    .collect(),
                posts_returned: self.posts_returned,
            })
        }
    }

    fn fake(n: usize) -> FakeFeed {
        fake_with_returned(n, n as u32)
    }

    fn fake_with_returned(n: usize, posts_returned: u32) -> FakeFeed {
        FakeFeed {
            n,
            posts_returned,
            seen: std::sync::Mutex::new(None),
        }
    }

    #[test]
    fn normalize_strips_at_and_falls_back_to_defaults() {
        let raw = vec![
            "@jensenhuang".to_string(),
            "  elonmusk ".to_string(),
            "".to_string(),
        ];
        assert_eq!(
            normalize_accounts(&raw).unwrap(),
            vec!["jensenhuang", "elonmusk"]
        );
        assert_eq!(
            normalize_accounts(&[]).unwrap(),
            DEFAULT_PULSE_ACCOUNTS.to_vec()
        );
    }

    #[test]
    fn normalize_mixed_valid_invalid_keeps_valid() {
        let raw = vec![
            "jensenhuang".to_string(),
            "jensen huang".to_string(), // space -> invalid
            "way_too_long_a_handle_over_15".to_string(), // > 15 chars -> invalid
            "elon-musk".to_string(),    // hyphen -> invalid
            "elonmusk".to_string(),
        ];
        assert_eq!(
            normalize_accounts(&raw).unwrap(),
            vec!["jensenhuang", "elonmusk"]
        );
    }

    #[test]
    fn normalize_all_invalid_nonempty_errors() {
        let raw = vec!["@".to_string(), "  ".to_string(), "bad handle".to_string()];
        let err = normalize_accounts(&raw).unwrap_err();
        assert!(matches!(err, DomainError::SourceFailure { ref name, .. } if name == "x"));
        assert!(err.to_string().contains("no valid X handles"));
    }

    #[tokio::test]
    async fn pulse_clamps_and_computes_cost() {
        let feed = fake(3);
        let report = pulse("nvda", &[], 500, 900, &feed, at()).await.unwrap();
        let (ticker, accounts, hours, limit) = feed.seen.lock().unwrap().clone().unwrap();
        assert_eq!(ticker, "NVDA"); // Ticker::parse normalizes
        assert_eq!(accounts, DEFAULT_PULSE_ACCOUNTS.to_vec());
        assert_eq!(hours, 167);
        assert_eq!(limit, 100);
        assert_eq!(report.posts_read, 3);
        assert!((report.estimated_cost_usd - 0.015).abs() < 1e-9);
        assert_eq!(report.generated_at, at());
    }

    #[tokio::test]
    async fn pulse_bills_what_x_returned_not_what_we_kept() {
        // Client-side truncation/skips kept 2 posts, but X returned (and
        // billed) 10 — e.g. the max_results floor of 10 with a low limit.
        let feed = fake_with_returned(2, 10);
        let report = pulse("AAPL", &[], 24, 2, &feed, at()).await.unwrap();
        assert_eq!(report.posts.len(), 2);
        assert_eq!(report.posts_read, 10);
        assert!((report.estimated_cost_usd - 0.05).abs() < 1e-9);
    }

    #[tokio::test]
    async fn pulse_clamps_low_bounds_and_zero_posts_is_ok() {
        let feed = fake(0);
        let report = pulse("AAPL", &["a".into()], 0, 0, &feed, at())
            .await
            .unwrap();
        let (_, _, hours, limit) = feed.seen.lock().unwrap().clone().unwrap();
        assert_eq!(hours, 1);
        assert_eq!(limit, 1);
        assert_eq!(report.posts_read, 0);
        assert_eq!(report.estimated_cost_usd, 0.0);
    }

    #[tokio::test]
    async fn pulse_rejects_all_invalid_handles_without_falling_back_to_defaults() {
        let feed = fake(0);
        let err = pulse("AAPL", &["bad handle".into()], 24, 20, &feed, at())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no valid X handles"));
        assert!(feed.seen.lock().unwrap().is_none()); // never reached the paid call
    }

    #[tokio::test]
    async fn pulse_rejects_bad_ticker() {
        let feed = fake(0);
        assert!(pulse("$$$", &[], 24, 20, &feed, at()).await.is_err());
    }
}
