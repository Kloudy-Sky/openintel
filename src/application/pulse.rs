use chrono::{DateTime, Utc};

use crate::domain::entities::pulse::PulseReport;
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::ports::influencer_feed::InfluencerFeed;

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

pub const MAX_HOURS_BACK: u32 = 168;
pub const MAX_PULSE_LIMIT: usize = 100;

/// Trim, strip a leading `@`, drop empties; empty result -> the default list.
pub fn normalize_accounts(raw: &[String]) -> Vec<String> {
    let cleaned: Vec<String> = raw
        .iter()
        .map(|a| a.trim().trim_start_matches('@').to_string())
        .filter(|a| !a.is_empty())
        .collect();
    if cleaned.is_empty() {
        DEFAULT_PULSE_ACCOUNTS
            .iter()
            .map(|s| s.to_string())
            .collect()
    } else {
        cleaned
    }
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
    let accounts = normalize_accounts(accounts_raw);
    let hours_back = hours_back.clamp(1, MAX_HOURS_BACK);
    let limit = limit.clamp(1, MAX_PULSE_LIMIT);
    let posts = feed.pulse(&ticker, &accounts, hours_back, limit).await?;
    let posts_read = posts.len() as u32;
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
    use crate::domain::entities::pulse::PulsePost;
    use crate::domain::entities::social_post::PostText;
    use async_trait::async_trait;
    use chrono::TimeZone;

    fn at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 16, 12, 0, 0).unwrap()
    }

    /// Records what it was called with; returns `n` canned posts.
    struct FakeFeed {
        n: usize,
        seen: std::sync::Mutex<Option<(String, Vec<String>, u32, usize)>>,
    }

    #[async_trait]
    impl InfluencerFeed for FakeFeed {
        async fn pulse(
            &self,
            ticker: &Ticker,
            accounts: &[String],
            hours_back: u32,
            limit: usize,
        ) -> Result<Vec<PulsePost>, DomainError> {
            *self.seen.lock().unwrap() = Some((
                ticker.as_str().to_string(),
                accounts.to_vec(),
                hours_back,
                limit,
            ));
            Ok((0..self.n)
                .map(|i| PulsePost {
                    id: format!("p{i}"),
                    author: "someone".into(),
                    text: PostText::parse("hello market").unwrap(),
                    created_at: at(),
                    engagement: 1,
                })
                .collect())
        }
    }

    fn fake(n: usize) -> FakeFeed {
        FakeFeed {
            n,
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
        assert_eq!(normalize_accounts(&raw), vec!["jensenhuang", "elonmusk"]);
        let empty: Vec<String> = vec!["@".to_string(), "  ".to_string()];
        assert_eq!(normalize_accounts(&empty), DEFAULT_PULSE_ACCOUNTS.to_vec());
        assert_eq!(normalize_accounts(&[]), DEFAULT_PULSE_ACCOUNTS.to_vec());
    }

    #[tokio::test]
    async fn pulse_clamps_and_computes_cost() {
        let feed = fake(3);
        let report = pulse("nvda", &[], 500, 900, &feed, at()).await.unwrap();
        let (ticker, accounts, hours, limit) = feed.seen.lock().unwrap().clone().unwrap();
        assert_eq!(ticker, "NVDA"); // Ticker::parse normalizes
        assert_eq!(accounts, DEFAULT_PULSE_ACCOUNTS.to_vec());
        assert_eq!(hours, 168);
        assert_eq!(limit, 100);
        assert_eq!(report.posts_read, 3);
        assert!((report.estimated_cost_usd - 0.015).abs() < 1e-9);
        assert_eq!(report.generated_at, at());
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
    async fn pulse_rejects_bad_ticker() {
        let feed = fake(0);
        assert!(pulse("$$$", &[], 24, 20, &feed, at()).await.is_err());
    }
}
