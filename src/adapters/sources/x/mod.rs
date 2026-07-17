mod response;

use std::time::Duration as StdDuration;

use async_trait::async_trait;
use chrono::{Duration, SecondsFormat, TimeZone, Utc};
use secrecy::{ExposeSecret, SecretString};

use crate::domain::entities::pulse::PulseFetch;
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::ports::influencer_feed::InfluencerFeed;

const SEARCH_URL: &str = "https://api.x.com/2/tweets/search/recent";
const TIMEOUT_SECS: u64 = 10;
/// X caps `search/recent` `query` at 512 chars on this tier.
const MAX_QUERY_CHARS: usize = 512;

/// Build the author-filtered search query.
/// High-impact accounts write company language ("Tesla", "Robotaxi"), not
/// cashtags — live-verified: 0 posts in 7 days from hourly-posting accounts
/// on a cashtag-only query. The symbol terms (`$TICKER`, `TICKER`) are always
/// present, bare (trusted — sourced from `Ticker`, not free text). Caller
/// -supplied keywords broaden recall onto that language and are each wrapped
/// in `"…"`: X's grammar treats a quoted string as a literal phrase, which
/// neutralizes operator interpretation (a keyword starting with `-` can't be
/// read as the NOT operator, `OR`/`from:` inside a keyword are just words)
/// and, as a side effect, allows multi-word phrases like "General Motors".
/// Contingency (spec): if pay-per-use rejects the `$` cashtag operator
/// (HTTP 400 naming the operator in the live test), drop the `$` prefix here.
pub(crate) fn build_query(ticker: &Ticker, accounts: &[String], keywords: &[String]) -> String {
    let from = accounts
        .iter()
        .map(|a| format!("from:{a}"))
        .collect::<Vec<_>>()
        .join(" OR ");
    let mut terms = vec![format!("${}", ticker.as_str()), ticker.as_str().to_string()];
    terms.extend(keywords.iter().filter_map(|k| {
        // Defense in depth: the application layer validates keywords, but this
        // is a pub adapter — strip quote chars so a raw caller can't break the
        // phrase grammar, and skip anything left empty.
        let clean = k.replace('"', "");
        let clean = clean.trim();
        (!clean.is_empty()).then(|| format!("\"{clean}\""))
    }));
    let terms = terms.join(" OR ");
    format!("({terms}) ({from}) -is:retweet")
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
        keywords: &[String],
        hours_back: u32,
        limit: usize,
    ) -> Result<PulseFetch, DomainError> {
        if limit == 0 {
            // No request made, nothing billed.
            return Ok(PulseFetch {
                posts: Vec::new(),
                posts_returned: 0,
            });
        }
        let fetched_at = Utc::now();
        let start_time = (fetched_at - Duration::hours(i64::from(hours_back)))
            .to_rfc3339_opts(SecondsFormat::Secs, true);
        let max_results = limit.clamp(10, 100).to_string(); // API minimum is 10

        let query = build_query(ticker, accounts, keywords);
        if query.chars().count() > MAX_QUERY_CHARS {
            return Err(fail(format!(
                "query too long ({} chars, max {MAX_QUERY_CHARS}) — use fewer accounts/keywords",
                query.chars().count()
            )));
        }

        // `.query()` is behind reqwest's un-enabled `query` feature; build manually.
        let mut url = reqwest::Url::parse(SEARCH_URL).map_err(|e| fail(format!("bad url: {e}")))?;
        url.query_pairs_mut()
            .append_pair("query", &query)
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
            build_query(&t, &accounts, &[]),
            "($NVDA OR NVDA) (from:jensenhuang OR from:elonmusk) -is:retweet"
        );
        let one = vec!["WhiteHouse".to_string()];
        assert_eq!(
            build_query(&t, &one, &[]),
            "($NVDA OR NVDA) (from:WhiteHouse) -is:retweet"
        );
    }

    #[test]
    fn build_query_appends_quoted_keywords_after_symbol_terms() {
        let t = Ticker::parse("TSLA").unwrap();
        let accounts = vec!["elonmusk".to_string()];
        let keywords = vec!["Tesla".to_string(), "Robotaxi".to_string()];
        assert_eq!(
            build_query(&t, &accounts, &keywords),
            "($TSLA OR TSLA OR \"Tesla\" OR \"Robotaxi\") (from:elonmusk) -is:retweet"
        );
    }

    #[test]
    fn build_query_quotes_leading_dash_keyword_so_it_cannot_act_as_not_operator() {
        // Regression: a bare `-recall` inside the OR group is X's NOT
        // operator and matches nearly everything, collapsing the ticker
        // filter. Quoting makes it a literal phrase instead.
        let t = Ticker::parse("TSLA").unwrap();
        let accounts = vec!["elonmusk".to_string()];
        let keywords = vec!["-recall".to_string()];
        assert_eq!(
            build_query(&t, &accounts, &keywords),
            "($TSLA OR TSLA OR \"-recall\") (from:elonmusk) -is:retweet"
        );
    }

    #[test]
    fn build_query_supports_multi_word_keyword_phrases() {
        // The motivating use case: "General Motors" is dropped by the
        // charset check if bare (spaces are invalid unquoted operators) but
        // survives as one quoted literal phrase.
        let t = Ticker::parse("GM").unwrap();
        let accounts = vec!["elonmusk".to_string()];
        let keywords = vec!["General Motors".to_string()];
        assert_eq!(
            build_query(&t, &accounts, &keywords),
            "($GM OR GM OR \"General Motors\") (from:elonmusk) -is:retweet"
        );
    }

    #[test]
    fn build_query_strips_embedded_quotes_and_skips_quote_only_keywords() {
        // Defense in depth: application::pulse::normalize_keywords is trusted to
        // reject `"`-bearing keywords, but XPulseSource is `pub` so an external
        // caller could bypass it. build_query must not let a raw `"` break the
        // phrase grammar.
        let t = Ticker::parse("TSLA").unwrap();
        let accounts = vec!["elonmusk".to_string()];
        let keywords = vec!["ha\"ha".to_string(), "\"\"\"".to_string()];
        assert_eq!(
            build_query(&t, &accounts, &keywords),
            "($TSLA OR TSLA OR \"haha\") (from:elonmusk) -is:retweet"
        );
    }

    #[test]
    fn new_builds() {
        assert!(XPulseSource::new(secret("token")).is_ok());
    }

    #[tokio::test]
    async fn pulse_rejects_oversized_query_before_any_network_io() {
        // The 512-char check runs before the HTTP client is touched, so this
        // is hermetic despite being a `#[tokio::test]` calling `.pulse()` —
        // no request is ever sent.
        let t = Ticker::parse("TSLA").unwrap();
        let accounts = vec!["elonmusk".to_string()];
        let keywords: Vec<String> = (0..60).map(|i| format!("keyword-number-{i}")).collect();
        let src = XPulseSource::new(secret("token")).unwrap();
        let err = src
            .pulse(&t, &accounts, &keywords, 24, 10)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("query too long"),
            "unexpected error: {err}"
        );
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
        let fetch = src
            .pulse(&Ticker::parse("AAPL").unwrap(), &accounts, &[], 167, 10)
            .await
            .unwrap(); // cashtag-operator contingency check: a 400 here means switch build_query to bare keyword
        for p in &fetch.posts {
            assert!(!p.id.is_empty());
            assert!(!p.text.as_str().is_empty());
        }
    }
}
