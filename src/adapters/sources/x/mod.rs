mod response;

use std::time::Duration as StdDuration;

use async_trait::async_trait;
use chrono::{Duration, SecondsFormat, TimeZone, Utc};
use secrecy::{ExposeSecret, SecretString};

use crate::domain::entities::pulse::PulsePost;
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::ports::influencer_feed::InfluencerFeed;

const SEARCH_URL: &str = "https://api.x.com/2/tweets/search/recent";
const TIMEOUT_SECS: u64 = 10;

/// Build the author-filtered search query.
/// Contingency (spec): if pay-per-use rejects the `$` cashtag operator
/// (HTTP 400 naming the operator in the live test), drop the `$` prefix here.
pub(crate) fn build_query(ticker: &Ticker, accounts: &[String]) -> String {
    let from = accounts
        .iter()
        .map(|a| format!("from:{a}"))
        .collect::<Vec<_>>()
        .join(" OR ");
    format!("${} ({from}) -is:retweet", ticker.as_str())
}

fn fail(message: impl Into<String>) -> DomainError {
    DomainError::SourceFailure {
        name: "x".into(),
        message: message.into(),
    }
}

pub struct XPulseSource {
    client: reqwest::Client,
    bearer: SecretString,
    user_agent: String,
}

impl XPulseSource {
    pub fn new(bearer: SecretString) -> Result<Self, DomainError> {
        let user_agent = format!("rust:openintel:v{}", env!("CARGO_PKG_VERSION"));
        let client = reqwest::Client::builder()
            .timeout(StdDuration::from_secs(TIMEOUT_SECS))
            .user_agent(&user_agent)
            .build()
            .map_err(|e| fail(format!("client build failed: {e}")))?;
        Ok(Self {
            client,
            bearer,
            user_agent,
        })
    }
}

#[async_trait]
impl InfluencerFeed for XPulseSource {
    async fn pulse(
        &self,
        ticker: &Ticker,
        accounts: &[String],
        hours_back: u32,
        limit: usize,
    ) -> Result<Vec<PulsePost>, DomainError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let fetched_at = Utc::now();
        let start_time = (fetched_at - Duration::hours(i64::from(hours_back)))
            .to_rfc3339_opts(SecondsFormat::Secs, true);
        let max_results = limit.clamp(10, 100).to_string(); // API minimum is 10

        // `.query()` is behind reqwest's un-enabled `query` feature; build manually.
        let mut url = reqwest::Url::parse(SEARCH_URL).map_err(|e| fail(format!("bad url: {e}")))?;
        url.query_pairs_mut()
            .append_pair("query", &build_query(ticker, accounts))
            .append_pair("start_time", &start_time)
            .append_pair("max_results", &max_results)
            .append_pair("tweet.fields", "created_at,public_metrics")
            .append_pair("expansions", "author_id")
            .append_pair("user.fields", "username");

        let resp = self
            .client
            .get(url)
            .bearer_auth(self.bearer.expose_secret())
            .header(reqwest::header::USER_AGENT, &self.user_agent)
            .send()
            .await
            .map_err(|e| fail(format!("search request failed: {e}")))?;
        let status = resp.status();
        let reset_hint = resp
            .headers()
            .get("x-rate-limit-reset")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<i64>().ok())
            .and_then(|secs| Utc.timestamp_opt(secs, 0).single());
        let body = resp
            .text()
            .await
            .map_err(|e| fail(format!("search body failed (HTTP {status}): {e}")))?;

        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(match reset_hint {
                Some(t) => fail(format!(
                    "rate limited (HTTP 429) — resets at {}",
                    t.to_rfc3339_opts(SecondsFormat::Secs, true)
                )),
                None => fail("rate limited (HTTP 429)"),
            });
        }
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(fail("unauthorized — check bearer token"));
        }
        if status == reqwest::StatusCode::FORBIDDEN {
            return Err(fail("forbidden — check API access and credit balance"));
        }
        if !status.is_success() {
            return Err(fail(format!("search HTTP {status}")));
        }
        response::parse_posts(&body, limit, fetched_at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret(s: &str) -> SecretString {
        SecretString::new(s.to_string().into_boxed_str())
    }

    #[test]
    fn build_query_shapes_cashtag_from_chain_and_retweet_filter() {
        let t = Ticker::parse("NVDA").unwrap();
        let accounts = vec!["jensenhuang".to_string(), "elonmusk".to_string()];
        assert_eq!(
            build_query(&t, &accounts),
            "$NVDA (from:jensenhuang OR from:elonmusk) -is:retweet"
        );
        let one = vec!["WhiteHouse".to_string()];
        assert_eq!(build_query(&t, &one), "$NVDA (from:WhiteHouse) -is:retweet");
    }

    #[test]
    fn new_builds() {
        assert!(XPulseSource::new(secret("token")).is_ok());
    }

    #[tokio::test]
    #[ignore = "hits live X (paid: ~10 post reads ≈ $0.05); needs OPENINTEL_X_BEARER; run with --ignored"]
    async fn x_live_pulse() {
        let bearer = std::env::var("OPENINTEL_X_BEARER").unwrap();
        let src = XPulseSource::new(SecretString::new(bearer.into_boxed_str())).unwrap();
        let accounts: Vec<String> = crate::application::pulse::DEFAULT_PULSE_ACCOUNTS
            .iter()
            .map(|s| s.to_string())
            .collect();
        let posts = src
            .pulse(&Ticker::parse("AAPL").unwrap(), &accounts, 168, 10)
            .await
            .unwrap(); // cashtag-operator contingency check: a 400 here means switch build_query to bare keyword
        for p in &posts {
            assert!(!p.id.is_empty());
            assert!(!p.text.as_str().is_empty());
        }
    }
}
