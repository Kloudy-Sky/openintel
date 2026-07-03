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

const SUBS: &str = "wallstreetbets+stocks+options+investing+StockMarket";
const API_BASE: &str = "https://oauth.reddit.com";
const TIMEOUT_SECS: u64 = 10;

pub struct RedditSource {
    client: reqwest::Client,
    client_id: SecretString,
    client_secret: SecretString,
    user_agent: String,
    token: RwLock<Option<CachedToken>>,
}

impl RedditSource {
    pub fn new(client_id: SecretString, client_secret: SecretString) -> Result<Self, DomainError> {
        let user_agent = format!(
            "rust:openintel:v{} (by /u/openintel)",
            env!("CARGO_PKG_VERSION")
        );
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(TIMEOUT_SECS))
            .user_agent(&user_agent)
            .build()
            .map_err(|e| DomainError::SourceFailure {
                name: "reddit".into(),
                message: format!("client build failed: {e}"),
            })?;
        Ok(Self {
            client,
            client_id,
            client_secret,
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
        let fresh = auth::request_token(
            &self.client,
            &self.client_id,
            &self.client_secret,
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
impl SocialDataSource for RedditSource {
    fn kind(&self) -> SourceKind {
        SourceKind::Reddit
    }

    async fn fetch(&self, ticker: &Ticker, limit: usize) -> Result<Vec<SocialPost>, DomainError> {
        let bearer = self.ensure_token().await?;
        let fetched_at = Utc::now();
        let cashtag = format!("${}", ticker.as_str());
        let limit_str = limit.min(100).to_string();
        // `.query()` is behind reqwest's `query` feature, which this crate does not enable;
        // build the query string manually via the re-exported `url::Url` instead.
        let mut url = reqwest::Url::parse(&format!("{API_BASE}/r/{SUBS}/search")).map_err(|e| {
            DomainError::SourceFailure {
                name: "reddit".into(),
                message: format!("bad search url: {e}"),
            }
        })?;
        url.query_pairs_mut()
            .append_pair("q", &cashtag)
            .append_pair("restrict_sr", "1")
            .append_pair("sort", "new")
            .append_pair("type", "link")
            .append_pair("limit", &limit_str)
            .append_pair("raw_json", "1");

        let resp = self
            .client
            .get(url)
            .bearer_auth(bearer.expose_secret())
            .header(reqwest::header::USER_AGENT, &self.user_agent)
            .send()
            .await
            .map_err(|e| DomainError::SourceFailure {
                name: "reddit".into(),
                message: format!("search request failed: {e}"),
            })?;
        let status = resp.status();
        let body = resp.text().await.map_err(|e| DomainError::SourceFailure {
            name: "reddit".into(),
            message: format!("search body failed (HTTP {status}): {e}"),
        })?;
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(DomainError::SourceFailure {
                name: "reddit".into(),
                message: "rate limited (HTTP 429)".into(),
            });
        }
        if !status.is_success() {
            return Err(DomainError::SourceFailure {
                name: "reddit".into(),
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
    fn new_builds_and_kind_is_reddit() {
        let src = RedditSource::new(secret("id"), secret("sec")).unwrap();
        assert_eq!(src.kind(), SourceKind::Reddit);
    }

    #[tokio::test]
    #[ignore = "hits live Reddit; needs OPENINTEL_REDDIT_CLIENT_ID/SECRET; run with --ignored"]
    async fn live_fetch_returns_posts() {
        let id = std::env::var("OPENINTEL_REDDIT_CLIENT_ID").unwrap();
        let secret = std::env::var("OPENINTEL_REDDIT_CLIENT_SECRET").unwrap();
        let src = RedditSource::new(
            SecretString::new(id.into_boxed_str()),
            SecretString::new(secret.into_boxed_str()),
        )
        .unwrap();
        let posts = src
            .fetch(&Ticker::parse("AAPL").unwrap(), 10)
            .await
            .unwrap();
        // A live search may legitimately return 0; assert the call succeeded and any post is well-formed.
        for p in &posts {
            assert!(!p.id.is_empty());
            assert!(!p.text.as_str().is_empty());
        }
    }
}
