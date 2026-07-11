mod auth;
mod response;

use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use secrecy::{ExposeSecret, SecretString};
use tokio::sync::RwLock;

use crate::domain::entities::social_post::SocialPost;
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::ports::social_data_source::SocialDataSource;
use crate::domain::values::source_kind::SourceKind;
use auth::CachedToken;

const PDS_BASE: &str = "https://bsky.social";
const TIMEOUT_SECS: u64 = 10;

pub struct BlueskySource {
    client: reqwest::Client,
    handle: String,
    app_password: SecretString,
    user_agent: String,
    token: RwLock<Option<CachedToken>>,
}

impl BlueskySource {
    pub fn new(handle: String, app_password: SecretString) -> Result<Self, DomainError> {
        let user_agent = format!("rust:openintel:v{}", env!("CARGO_PKG_VERSION"));
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(TIMEOUT_SECS))
            .user_agent(&user_agent)
            .build()
            .map_err(|e| DomainError::SourceFailure {
                name: "bluesky".into(),
                message: format!("client build failed: {e}"),
            })?;
        Ok(Self {
            client,
            handle,
            app_password,
            user_agent,
            token: RwLock::new(None),
        })
    }

    async fn ensure_token(&self) -> Result<SecretString, DomainError> {
        let now = Utc::now();
        {
            let guard = self.token.read().await;
            if let Some(t) = guard.as_ref() {
                if !t.is_expired(now) {
                    return Ok(t.bearer.clone());
                }
            }
        }
        let mut guard = self.token.write().await;
        if let Some(t) = guard.as_ref() {
            if !t.is_expired(now) {
                return Ok(t.bearer.clone());
            }
        }
        let fresh = auth::request_session(
            &self.client,
            &self.handle,
            &self.app_password,
            &self.user_agent,
            now,
        )
        .await?;
        let bearer = fresh.bearer.clone();
        *guard = Some(fresh);
        Ok(bearer)
    }
}

#[async_trait]
impl SocialDataSource for BlueskySource {
    fn kind(&self) -> SourceKind {
        SourceKind::Bluesky
    }

    async fn fetch(&self, ticker: &Ticker, limit: usize) -> Result<Vec<SocialPost>, DomainError> {
        if limit == 0 {
            // searchPosts rejects limit=0 (lexicon minimum is 1); skip the round trip.
            return Ok(Vec::new());
        }
        let bearer = self.ensure_token().await?;
        let fetched_at = Utc::now();
        let limit_str = limit.min(100).to_string();
        // `.query()` is behind reqwest's un-enabled `query` feature; build manually.
        let mut url = reqwest::Url::parse(&format!("{PDS_BASE}/xrpc/app.bsky.feed.searchPosts"))
            .map_err(|e| DomainError::SourceFailure {
                name: "bluesky".into(),
                message: format!("bad search url: {e}"),
            })?;
        url.query_pairs_mut()
            .append_pair("q", ticker.as_str())
            .append_pair("sort", "latest")
            .append_pair("limit", &limit_str);

        let resp = self
            .client
            .get(url)
            .bearer_auth(bearer.expose_secret())
            .header(reqwest::header::USER_AGENT, &self.user_agent)
            .send()
            .await
            .map_err(|e| DomainError::SourceFailure {
                name: "bluesky".into(),
                message: format!("search request failed: {e}"),
            })?;
        let status = resp.status();
        let body = resp.text().await.map_err(|e| DomainError::SourceFailure {
            name: "bluesky".into(),
            message: format!("search body failed (HTTP {status}): {e}"),
        })?;
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(DomainError::SourceFailure {
                name: "bluesky".into(),
                message: "rate limited (HTTP 429)".into(),
            });
        }
        // atproto surfaces expired/invalid tokens as 400 (ExpiredToken/InvalidToken) or 401.
        if status == reqwest::StatusCode::BAD_REQUEST || status == reqwest::StatusCode::UNAUTHORIZED
        {
            return Err(DomainError::SourceFailure {
                name: "bluesky".into(),
                message: "unauthorized — check handle/app password".into(),
            });
        }
        if !status.is_success() {
            return Err(DomainError::SourceFailure {
                name: "bluesky".into(),
                message: format!("search HTTP {status}"),
            });
        }
        response::parse_posts(&body, limit, fetched_at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::SecretString;

    fn secret(s: &str) -> SecretString {
        SecretString::new(s.to_string().into_boxed_str())
    }

    #[test]
    fn new_builds_and_kind_is_bluesky() {
        let src = BlueskySource::new("someone.bsky.social".into(), secret("pw")).unwrap();
        assert_eq!(src.kind(), SourceKind::Bluesky);
    }

    #[tokio::test]
    #[ignore = "hits live Bluesky; needs OPENINTEL_BLUESKY_HANDLE/APP_PASSWORD; run with --ignored"]
    async fn live_fetch_returns_posts() {
        let handle = std::env::var("OPENINTEL_BLUESKY_HANDLE").unwrap();
        let pw = std::env::var("OPENINTEL_BLUESKY_APP_PASSWORD").unwrap();
        let src = BlueskySource::new(handle, SecretString::new(pw.into_boxed_str())).unwrap();
        let posts = src
            .fetch(&Ticker::parse("AAPL").unwrap(), 10)
            .await
            .unwrap();
        // A live search may legitimately return 0; assert the call succeeded and posts are well-formed.
        for p in &posts {
            assert!(!p.id.is_empty());
            assert!(!p.text.as_str().is_empty());
        }
    }

    #[tokio::test]
    async fn fetch_limit_zero_is_empty_without_network() {
        let src = BlueskySource::new("someone.bsky.social".into(), secret("pw")).unwrap();
        let posts = src.fetch(&Ticker::parse("AAPL").unwrap(), 0).await.unwrap();
        assert!(posts.is_empty()); // would error if it hit the network with fake creds
    }
}
