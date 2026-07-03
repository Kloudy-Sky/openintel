# Reddit OAuth Social Adapter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a real Reddit `SocialDataSource` (app-only OAuth over finance subreddits), injected via DI, replacing the mock Reddit fixtures in production while keeping tests hermetic.

**Architecture:** A pure `parse_posts` maps Reddit search JSON to `SocialPost`; `auth.rs` holds the cached-token OAuth (`client_credentials`); `RedditSource` (`mod.rs`) does the token + search HTTP and delegates to the parser. Social sources become an injected `&[Box<dyn SocialDataSource>]`; the composition roots build the list (Reddit only when creds are set), tests inject mocks.

**Tech Stack:** Rust, tokio, reqwest (rustls, already a dependency), serde_json, chrono, secrecy.

## Global Constraints

- Reddit access is **app-only OAuth `client_credentials`** — verified live that keyless Reddit 403s. Token: `POST https://www.reddit.com/api/v1/access_token`, body `grant_type=client_credentials`, HTTP Basic (`client_id`:`client_secret`); API base `https://oauth.reddit.com`, `Authorization: bearer <token>`.
- **User-Agent is mandatory** and must be descriptive: `rust:openintel:v{CARGO_PKG_VERSION} (by /u/openintel)` on every Reddit request (token + search). Generic UAs are throttled.
- Secrets are env-only via `secrecy::SecretString`: `OPENINTEL_REDDIT_CLIENT_ID`, `OPENINTEL_REDDIT_CLIENT_SECRET` (replacing the unused `OPENINTEL_REDDIT_TOKEN`). Never logged, never written to disk; `.expose_secret()` only at the HTTP call.
- All failures map to `DomainError::SourceFailure { name: "reddit".into(), message }` — no new error variant. **No `unwrap`/`expect` on network data.**
- Clock at the edge: `fetch`/`ensure_token` read `Utc::now()` and pass it into the pure `parse_posts` / `parse_token`; those are deterministic.
- `cargo test` must be **hermetic** — the only live test is `#[ignore]`d and reads creds from env.
- Bearer token is cached (`RwLock`) and reused across a `scan_watchlist`; refreshed 60s before `expires_in`.
- Graceful absence: no creds → Reddit source not wired → enabling reddit yields a `"reddit enabled but not configured"` note; other sources + market still run.
- YAGNI: submissions only (no comments), fixed subreddit set, no retry/backoff, no OS keychain.
- Each task passes `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`.

---

## File Structure

**Create**
- `src/adapters/sources/reddit/response.rs` — serde DTOs + pure `parse_posts` + tests.
- `src/adapters/sources/reddit/auth.rs` — `CachedToken`, `parse_token`, `request_token` + tests.
- `src/adapters/sources/reddit/mod.rs` — `RedditSource` (HTTP shell + `impl SocialDataSource`).

**Modify**
- `src/adapters/sources/mod.rs` — `pub mod reddit;`.
- `src/config/secrets.rs` — Reddit client id/secret.
- `src/application/analyze.rs` — inject social sources; drop `build_sources`.
- `src/cli/run.rs`, `src/main.rs` — build + inject the social list.
- `src/mcp/tools.rs`, `src/mcp/server.rs` — thread + own the social list.
- `tests/analyze_flow.rs` — inject the mock social list.
- `README.md` — Reddit setup + secret guidance.

---

## Task 1: Pure Reddit response parser

**Files:**
- Create: `src/adapters/sources/reddit/mod.rs` (this task: only `mod response;`)
- Create: `src/adapters/sources/reddit/response.rs`
- Modify: `src/adapters/sources/mod.rs`
- Test: unit tests in `response.rs`

**Interfaces:**
- Consumes: `SocialPost`, `PostText` (`src/domain/entities/social_post.rs`); `SourceKind::Reddit`; `DomainError::SourceFailure { name, message }`.
- Produces: `pub(crate) fn parse_posts(body: &str, limit: usize, fetched_at: DateTime<Utc>) -> Result<Vec<SocialPost>, DomainError>`.

- [ ] **Step 1: Register the module tree**

`src/adapters/sources/mod.rs` — add `pub mod reddit;` (keep existing `pub mod mock_reddit;` etc.).

Create `src/adapters/sources/reddit/mod.rs` with only:

```rust
mod response;
```

- [ ] **Step 2: Write the failing tests**

Create `src/adapters/sources/reddit/response.rs` with this test module at the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 2, 0, 0, 0).unwrap()
    }

    const HAPPY: &str = r#"{"kind":"Listing","data":{"children":[
        {"kind":"t3","data":{"name":"t3_aaa","author":"wsbtrader","title":"$AAPL calls printing","selftext":"loading more","score":420,"created_utc":1782504000.0}},
        {"kind":"t3","data":{"name":"t3_bbb","author":"[deleted]","title":"AAPL puts","selftext":"","score":-5,"created_utc":1782500000.0}}
    ]}}"#;

    const EMPTY: &str = r#"{"kind":"Listing","data":{"children":[]}}"#;

    #[test]
    fn happy_maps_posts() {
        let posts = parse_posts(HAPPY, 50, at()).unwrap();
        assert_eq!(posts.len(), 2);
        assert_eq!(posts[0].id, "t3_aaa");
        assert_eq!(posts[0].author, "wsbtrader");
        assert_eq!(posts[0].text.as_str(), "$AAPL calls printing\nloading more");
        assert_eq!(posts[0].engagement, 420);
        assert_eq!(posts[0].source, SourceKind::Reddit);
        assert_eq!(
            posts[0].created_at,
            Utc.timestamp_opt(1782504000, 0).single().unwrap()
        );
    }

    #[test]
    fn empty_selftext_is_title_only_and_deleted_author_kept() {
        let posts = parse_posts(HAPPY, 50, at()).unwrap();
        assert_eq!(posts[1].text.as_str(), "AAPL puts");
        assert_eq!(posts[1].author, "[deleted]");
    }

    #[test]
    fn negative_score_clamps_to_zero() {
        let posts = parse_posts(HAPPY, 50, at()).unwrap();
        assert_eq!(posts[1].engagement, 0);
    }

    #[test]
    fn limit_is_honored() {
        assert_eq!(parse_posts(HAPPY, 1, at()).unwrap().len(), 1);
    }

    #[test]
    fn empty_children_is_empty() {
        assert!(parse_posts(EMPTY, 50, at()).unwrap().is_empty());
    }

    #[test]
    fn missing_created_utc_falls_back_to_fetched_at() {
        let body = r#"{"kind":"Listing","data":{"children":[
            {"kind":"t3","data":{"name":"t3_c","author":"a","title":"AAPL","score":1}}
        ]}}"#;
        assert_eq!(parse_posts(body, 50, at()).unwrap()[0].created_at, at());
    }

    #[test]
    fn overlong_text_is_truncated() {
        let big = "A".repeat(20_000);
        let body = format!(
            r#"{{"kind":"Listing","data":{{"children":[{{"kind":"t3","data":{{"name":"t3_d","author":"a","title":"{big}","score":1,"created_utc":1.0}}}}]}}}}"#
        );
        let posts = parse_posts(&body, 50, at()).unwrap();
        assert_eq!(posts[0].text.as_str().chars().count(), 10_000);
    }

    #[test]
    fn post_with_no_id_is_skipped() {
        let body = r#"{"kind":"Listing","data":{"children":[
            {"kind":"t3","data":{"author":"a","title":"AAPL","score":1}}
        ]}}"#;
        assert!(parse_posts(body, 50, at()).unwrap().is_empty());
    }

    #[test]
    fn malformed_json_is_source_failure() {
        assert!(parse_posts("not json", 50, at()).is_err());
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p openintel --lib adapters::sources::reddit`
Expected: FAIL to compile — `parse_posts` not found.

- [ ] **Step 4: Write the implementation**

Prepend to `src/adapters/sources/reddit/response.rs` (above the tests):

```rust
#![allow(dead_code)]

use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;

use crate::domain::entities::social_post::{PostText, SocialPost};
use crate::domain::error::DomainError;
use crate::domain::values::source_kind::SourceKind;

const MAX_TEXT_CHARS: usize = 10_000;

#[derive(Debug, Deserialize)]
struct Listing {
    data: ListingData,
}

#[derive(Debug, Deserialize)]
struct ListingData {
    #[serde(default)]
    children: Vec<Child>,
}

#[derive(Debug, Deserialize)]
struct Child {
    data: ChildData,
}

#[derive(Debug, Deserialize)]
struct ChildData {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    selftext: Option<String>,
    #[serde(default)]
    score: Option<i64>,
    #[serde(default)]
    created_utc: Option<f64>,
}

fn fail(message: impl Into<String>) -> DomainError {
    DomainError::SourceFailure {
        name: "reddit".into(),
        message: message.into(),
    }
}

pub(crate) fn parse_posts(
    body: &str,
    limit: usize,
    fetched_at: DateTime<Utc>,
) -> Result<Vec<SocialPost>, DomainError> {
    let listing: Listing =
        serde_json::from_str(body).map_err(|e| fail(format!("malformed response: {e}")))?;

    let mut posts = Vec::new();
    for child in listing.data.children {
        let d = child.data;
        let id = match d.name.or(d.id) {
            Some(id) if !id.is_empty() => id,
            _ => continue,
        };
        let title = d.title.unwrap_or_default();
        let selftext = d.selftext.unwrap_or_default();
        let combined = if selftext.trim().is_empty() {
            title
        } else {
            format!("{title}\n{selftext}")
        };
        let truncated: String = combined.chars().take(MAX_TEXT_CHARS).collect();
        let text = match PostText::parse(&truncated) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let created_at = d
            .created_utc
            .and_then(|s| Utc.timestamp_opt(s as i64, 0).single())
            .unwrap_or(fetched_at);

        posts.push(SocialPost {
            id,
            source: SourceKind::Reddit,
            author: d.author.unwrap_or_else(|| "[unknown]".to_string()),
            text,
            created_at,
            engagement: d.score.unwrap_or(0).max(0) as u32,
        });
        if posts.len() >= limit {
            break;
        }
    }
    Ok(posts)
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p openintel --lib adapters::sources::reddit`
Expected: PASS (9 tests).

- [ ] **Step 6: Lint + format**

Run: `cargo clippy --all-targets -- -D warnings && cargo fmt`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add src/adapters/sources/mod.rs src/adapters/sources/reddit/
git commit -m "feat(reddit): pure Reddit search response parser"
```

---

## Task 2: Reddit OAuth source (auth + HTTP fetch)

**Files:**
- Create: `src/adapters/sources/reddit/auth.rs`
- Modify: `src/adapters/sources/reddit/mod.rs` (add `RedditSource`; remove the Task-1 `#![allow(dead_code)]` from `response.rs`)
- Test: unit tests in `auth.rs`; unit + `#[ignore]` live test in `mod.rs`

**Interfaces:**
- Consumes: `response::parse_posts` (Task 1); `SocialDataSource` port (`fn kind(&self) -> SourceKind`, `async fn fetch(&self, ticker: &Ticker, limit: usize) -> Result<Vec<SocialPost>, DomainError>`); `secrecy::{SecretString, ExposeSecret}`.
- Produces:
  - `auth.rs` (`pub(crate)`): `CachedToken { bearer: SecretString, expires_at: DateTime<Utc> }` with `is_expired(&self, now: DateTime<Utc>) -> bool`; `parse_token(body: &str, now: DateTime<Utc>) -> Result<CachedToken, DomainError>`; `async fn request_token(client, client_id, client_secret, user_agent, now) -> Result<CachedToken, DomainError>`.
  - `mod.rs`: `RedditSource` with `pub fn new(client_id: SecretString, client_secret: SecretString) -> Result<Self, DomainError>`; `impl SocialDataSource` (`kind()` → `Reddit`).

- [ ] **Step 1: Write the failing tests for auth**

Create `src/adapters/sources/reddit/auth.rs` with this test module at the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 2, 0, 0, 0).unwrap()
    }

    #[test]
    fn parse_token_ok_and_expiry() {
        let body = r#"{"access_token":"abc123","token_type":"bearer","expires_in":3600,"scope":"*"}"#;
        let t = parse_token(body, at()).unwrap();
        assert_eq!(t.bearer.expose_secret(), "abc123");
        assert!(!t.is_expired(at())); // fresh
        assert!(t.is_expired(at() + Duration::seconds(3600))); // at expiry
        assert!(t.is_expired(at() + Duration::seconds(3541))); // 60s skew -> refresh early
        assert!(!t.is_expired(at() + Duration::seconds(3539)));
    }

    #[test]
    fn parse_token_error_field_is_failure() {
        let body = r#"{"error":"invalid_grant"}"#;
        assert!(parse_token(body, at()).is_err());
    }

    #[test]
    fn parse_token_missing_access_token_is_failure() {
        let body = r#"{"token_type":"bearer","expires_in":3600}"#;
        assert!(parse_token(body, at()).is_err());
    }

    #[test]
    fn parse_token_malformed_is_failure() {
        assert!(parse_token("nope", at()).is_err());
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p openintel --lib adapters::sources::reddit::auth`
Expected: FAIL to compile — `parse_token` / `CachedToken` not found.

- [ ] **Step 3: Implement auth.rs**

Prepend to `src/adapters/sources/reddit/auth.rs`:

```rust
use chrono::{DateTime, Duration, Utc};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

use crate::domain::error::DomainError;

const SKEW_SECS: i64 = 60;
const TOKEN_URL: &str = "https://www.reddit.com/api/v1/access_token";

pub(crate) struct CachedToken {
    pub bearer: SecretString,
    pub expires_at: DateTime<Utc>,
}

impl CachedToken {
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        now + Duration::seconds(SKEW_SECS) >= self.expires_at
    }
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    error: Option<String>,
}

fn fail(message: impl Into<String>) -> DomainError {
    DomainError::SourceFailure {
        name: "reddit".into(),
        message: message.into(),
    }
}

pub(crate) fn parse_token(body: &str, now: DateTime<Utc>) -> Result<CachedToken, DomainError> {
    let resp: TokenResponse =
        serde_json::from_str(body).map_err(|e| fail(format!("malformed token response: {e}")))?;
    if let Some(err) = resp.error {
        return Err(fail(format!("token error: {err}")));
    }
    let access_token = resp
        .access_token
        .filter(|t| !t.is_empty())
        .ok_or_else(|| fail("no access_token in response"))?;
    let expires_in = resp.expires_in.unwrap_or(3600);
    Ok(CachedToken {
        bearer: SecretString::new(access_token.into_boxed_str()),
        expires_at: now + Duration::seconds(expires_in),
    })
}

pub(crate) async fn request_token(
    client: &reqwest::Client,
    client_id: &SecretString,
    client_secret: &SecretString,
    user_agent: &str,
    now: DateTime<Utc>,
) -> Result<CachedToken, DomainError> {
    let resp = client
        .post(TOKEN_URL)
        .basic_auth(client_id.expose_secret(), Some(client_secret.expose_secret()))
        .header(reqwest::header::USER_AGENT, user_agent)
        .form(&[("grant_type", "client_credentials")])
        .send()
        .await
        .map_err(|e| fail(format!("token request failed: {e}")))?;
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| fail(format!("token body failed (HTTP {status}): {e}")))?;
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(fail("unauthorized — check client id/secret"));
    }
    if !status.is_success() {
        return Err(fail(format!("token request HTTP {status}")));
    }
    parse_token(&body, now)
}
```

> If `.form(...)` fails to compile under the current `reqwest` features, replace it with `.header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded").body("grant_type=client_credentials")` — no feature needed. (`.basic_auth`, `.bearer_auth`, `.query`, `.form` are all part of reqwest's always-compiled `RequestBuilder`; only `.json` is behind the `json` feature.)

- [ ] **Step 4: Run auth tests**

Run: `cargo test -p openintel --lib adapters::sources::reddit::auth`
Expected: PASS (4 tests).

- [ ] **Step 5: Write the RedditSource tests**

Add this test module at the bottom of `src/adapters/sources/reddit/mod.rs`:

```rust
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
```

- [ ] **Step 6: Run to verify it fails**

Run: `cargo test -p openintel --lib adapters::sources::reddit::tests::new_builds_and_kind_is_reddit`
Expected: FAIL to compile — `RedditSource` not found.

- [ ] **Step 7: Implement RedditSource + remove the Task-1 allow**

Replace the top of `src/adapters/sources/reddit/mod.rs` (keeping the Step-5 test module at the bottom) so it reads:

```rust
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
        let url = format!("{API_BASE}/r/{SUBS}/search");

        let resp = self
            .client
            .get(url)
            .query(&[
                ("q", cashtag.as_str()),
                ("restrict_sr", "1"),
                ("sort", "new"),
                ("type", "link"),
                ("limit", limit_str.as_str()),
                ("raw_json", "1"),
            ])
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
```

Then delete the `#![allow(dead_code)]` line at the top of `src/adapters/sources/reddit/response.rs` — `parse_posts` is now consumed by `fetch`, so it is live in the lib target (the Step 9 clippy run confirms no dead-code warnings).

- [ ] **Step 8: Run tests**

Run: `cargo test -p openintel --lib adapters::sources::reddit`
Expected: PASS (14 non-ignored: 9 parser + 4 auth + `new_builds…`; the live test is ignored).

- [ ] **Step 9: Lint + format**

Run: `cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: clean (no dead-code warning now that the allow is removed).

- [ ] **Step 10: Commit**

```bash
git add src/adapters/sources/reddit/
git commit -m "feat(reddit): app-only OAuth source with cached token + search"
```

---

## Task 3: Reddit client credentials in config

**Files:**
- Modify: `src/config/secrets.rs`
- Test: unit tests in `secrets.rs`

**Interfaces:**
- Produces: `Credentials { reddit_client_id: Option<SecretString>, reddit_client_secret: Option<SecretString>, x_bearer, bluesky_app_password, market_api_key }`, read from `OPENINTEL_REDDIT_CLIENT_ID` / `OPENINTEL_REDDIT_CLIENT_SECRET`.

- [ ] **Step 1: Update the struct + `from_env`**

In `src/config/secrets.rs`, replace the `reddit_token` field and its `from_env` line so the struct reads:

```rust
#[derive(Debug)]
pub struct Credentials {
    pub reddit_client_id: Option<SecretString>,
    pub reddit_client_secret: Option<SecretString>,
    pub x_bearer: Option<SecretString>,
    pub bluesky_app_password: Option<SecretString>,
    pub market_api_key: Option<SecretString>,
}

impl Credentials {
    pub fn from_env() -> Self {
        Credentials {
            reddit_client_id: secret_from(std::env::var("OPENINTEL_REDDIT_CLIENT_ID").ok()),
            reddit_client_secret: secret_from(std::env::var("OPENINTEL_REDDIT_CLIENT_SECRET").ok()),
            x_bearer: secret_from(std::env::var("OPENINTEL_X_BEARER").ok()),
            bluesky_app_password: secret_from(std::env::var("OPENINTEL_BLUESKY_APP_PASSWORD").ok()),
            market_api_key: secret_from(std::env::var("OPENINTEL_MARKET_API_KEY").ok()),
        }
    }
}
```

- [ ] **Step 2: Update the leak test**

In the `#[cfg(test)] mod tests`, update `debug_does_not_leak_secret` to use the new field:

```rust
    #[test]
    fn debug_does_not_leak_secret() {
        let creds = Credentials {
            reddit_client_id: secret_from(Some("leak-me".to_string())),
            reddit_client_secret: None,
            x_bearer: None,
            bluesky_app_password: None,
            market_api_key: None,
        };
        assert!(!format!("{creds:?}").contains("leak-me"));
    }
```

(The `wraps_present_value_and_skips_absent` test uses `secret_from` directly and needs no change.)

- [ ] **Step 3: Run tests**

Run: `cargo test -p openintel --lib config::secrets`
Expected: PASS (2 tests).

- [ ] **Step 4: Lint + format + commit**

Run: `cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: clean.

```bash
git add src/config/secrets.rs
git commit -m "feat(config): Reddit client id/secret env credentials"
```

---

## Task 4: Inject social sources; wire Reddit at composition roots

Pure DI refactor mirroring the market-source injection: thread `social_sources: &[Box<dyn SocialDataSource>]` through the analysis path, build the list at the entry points (Reddit only when both creds are set), and inject mocks in tests. Behavior on mock input is unchanged.

**Files:**
- Modify: `src/application/analyze.rs`, `src/cli/run.rs`, `src/main.rs`, `src/mcp/tools.rs`, `src/mcp/server.rs`, `tests/analyze_flow.rs`

**Interfaces:**
- Consumes: `RedditSource::new(...) -> Result<Self, DomainError>` (Task 2); `Credentials` fields (Task 3); the existing `MockRedditSource`/`MockXSource`/`MockBlueskySource` (unit structs, `impl SocialDataSource`).
- Produces (new signatures):
  - `application::analyze(req: &AnalysisRequest, social_sources: &[Box<dyn SocialDataSource>], market_source: Option<&dyn MarketDataSource>) -> Result<SpeculationReport, DomainError>`
  - `cli::run::analyze(config: &AppConfig, social_sources: &[Box<dyn SocialDataSource>], market_source: Option<&dyn MarketDataSource>) -> Result<(SpeculationReport, String), DomainError>`
  - `mcp::tools::run_list_sources(social_sources: &[Box<dyn SocialDataSource>], market_source: &dyn MarketDataSource) -> SourcesOutput`
  - `mcp::tools::run_analyze / run_scan / run_compare(args, social_sources: &[Box<dyn SocialDataSource>], market_source: &dyn MarketDataSource)`
  - `mcp::server::OpenIntelServer::new(social: Vec<Box<dyn SocialDataSource>>, market: YahooMarketSource)` (field `social: std::sync::Arc<Vec<Box<dyn SocialDataSource>>>`)

- [ ] **Step 1: `application::analyze` — inject social sources, drop `build_sources`**

In `src/application/analyze.rs`: delete the `build_sources` function and the mock-source `use` lines it needed (`MockBlueskySource`, `MockRedditSource`, `MockXSource`). Add `use crate::domain::ports::social_data_source::SocialDataSource;` if not present. Change `analyze`:

```rust
pub async fn analyze(
    req: &AnalysisRequest,
    social_sources: &[Box<dyn SocialDataSource>],
    market_source: Option<&dyn MarketDataSource>,
) -> Result<SpeculationReport, DomainError> {
    let ticker = Ticker::parse(&req.ticker)?;

    let mut notes: Vec<String> = Vec::new();
    for kind in &req.enabled_sources {
        if !social_sources.iter().any(|s| s.kind() == *kind) {
            notes.push(format!("{} enabled but not configured", kind.as_str()));
        }
    }

    let fetches = social_sources
        .iter()
        .filter(|s| req.enabled_sources.contains(&s.kind()))
        .map(|source| {
            let ticker = ticker.clone();
            async move { (source.kind(), source.fetch(&ticker, req.limit).await) }
        });
    let results = join_all(fetches).await;

    let mut posts: Vec<SocialPost> = Vec::new();
    for (kind, result) in results {
        match result {
            Ok(mut fetched) => posts.append(&mut fetched),
            Err(e) => notes.push(format!("source {} failed: {e}", kind.as_str())),
        }
    }

    let market: Option<MarketSnapshot> = match (req.market_enabled, market_source) {
        (true, Some(source)) => match source.snapshot(&ticker).await {
            Ok(snapshot) => Some(snapshot),
            Err(e) => {
                notes.push(format!("market source failed: {e}"));
                None
            }
        },
        _ => None,
    };

    if posts.is_empty() && market.is_none() {
        return Err(DomainError::NoData);
    }

    let analyzer = LexiconAnalyzer::new();
    let signals = analyzer.analyze(&posts).await?;

    let now = Utc::now();
    let mut report =
        SpeculationEngine::aggregate(&ticker, &posts, &signals, market.as_ref(), now, &req.engine)?;

    let engine_notes = std::mem::take(&mut report.fusion.notes);
    report.fusion.notes = notes.into_iter().chain(engine_notes).collect();

    Ok(report)
}
```

Update the `#[cfg(test)] mod tests`: add a mock-list helper and pass it. Replace the test module's helpers/calls:

```rust
    use crate::adapters::market::mock_market::MockMarketSource;
    use crate::adapters::sources::mock_bluesky::MockBlueskySource;
    use crate::adapters::sources::mock_reddit::MockRedditSource;
    use crate::adapters::sources::mock_x::MockXSource;

    fn mock_social() -> Vec<Box<dyn SocialDataSource>> {
        vec![
            Box::new(MockRedditSource),
            Box::new(MockXSource),
            Box::new(MockBlueskySource),
        ]
    }

    #[tokio::test]
    async fn analyzes_default_request_confirming_bullish() {
        use crate::domain::values::speculation::Alignment;
        let report = analyze(&req("AAPL", true), &mock_social(), Some(&MockMarketSource))
            .await
            .unwrap();
        assert_eq!(report.social.total_mentions, 10);
        assert_eq!(report.fusion.alignment, Alignment::ConfirmingBullish);
        assert!(report.market.is_some());
    }

    #[tokio::test]
    async fn invalid_ticker_errors() {
        assert!(
            analyze(&req("$$$", true), &mock_social(), Some(&MockMarketSource))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn social_only_when_no_source_provided() {
        use crate::domain::values::speculation::Alignment;
        let report = analyze(&req("AAPL", false), &mock_social(), None)
            .await
            .unwrap();
        assert!(report.market.is_none());
        assert_eq!(report.fusion.alignment, Alignment::Quiet);
    }

    #[tokio::test]
    async fn enabled_source_absent_is_noted() {
        // reddit enabled but not wired -> note + other sources still counted (x=3, bluesky=3)
        let social: Vec<Box<dyn SocialDataSource>> =
            vec![Box::new(MockXSource), Box::new(MockBlueskySource)];
        let report = analyze(&req("AAPL", false), &social, None).await.unwrap();
        assert_eq!(report.social.total_mentions, 6);
        assert!(report
            .fusion
            .notes
            .iter()
            .any(|n| n.contains("reddit enabled but not configured")));
    }
```

- [ ] **Step 2: `cli::run::analyze` — inject and forward**

In `src/cli/run.rs`: add `use crate::domain::ports::social_data_source::SocialDataSource;`. Change the signature + forward:

```rust
pub async fn analyze(
    config: &AppConfig,
    social_sources: &[Box<dyn SocialDataSource>],
    market_source: Option<&dyn MarketDataSource>,
) -> Result<(SpeculationReport, String), DomainError> {
    let req = AnalysisRequest {
        ticker: config.ticker.clone(),
        enabled_sources: config.enabled_sources.clone(),
        market_enabled: config.market_enabled,
        limit: config.limit,
        engine: config.engine.clone(),
    };
    let report = application::analyze(&req, social_sources, market_source).await?;
    let rendered = render(&report, config.format);
    Ok((report, rendered))
}
```

Update its tests: add the same `mock_social()` helper and thread it. The market-enabled cases pass `Some(&MockMarketSource)`, the `no_market` case passes `None`:

```rust
    use crate::adapters::market::mock_market::MockMarketSource;
    use crate::adapters::sources::mock_bluesky::MockBlueskySource;
    use crate::adapters::sources::mock_reddit::MockRedditSource;
    use crate::adapters::sources::mock_x::MockXSource;

    fn mock_social() -> Vec<Box<dyn SocialDataSource>> {
        vec![
            Box::new(MockRedditSource),
            Box::new(MockXSource),
            Box::new(MockBlueskySource),
        ]
    }

    #[tokio::test]
    async fn full_run_confirms_bullish_with_market() {
        let (report, rendered) = analyze(
            &config(false, OutputFormat::Json),
            &mock_social(),
            Some(&MockMarketSource),
        )
        .await
        .unwrap();
        assert!(report.market.is_some());
        assert_eq!(report.fusion.alignment, Alignment::ConfirmingBullish);
        assert!(rendered.contains("Not financial advice"));
        assert!(rendered.contains("speculation_index"));
    }

    #[tokio::test]
    async fn no_market_run_is_quiet() {
        let (report, _) = analyze(&config(true, OutputFormat::Table), &mock_social(), None)
            .await
            .unwrap();
        assert!(report.market.is_none());
        assert_eq!(report.fusion.alignment, Alignment::Quiet);
    }

    #[tokio::test]
    async fn table_output_has_sections_and_disclaimer() {
        let (_, rendered) = analyze(
            &config(false, OutputFormat::Table),
            &mock_social(),
            Some(&MockMarketSource),
        )
        .await
        .unwrap();
        assert!(rendered.contains("SOCIAL"));
        assert!(rendered.contains("MARKET"));
        assert!(rendered.contains("FUSION"));
        assert!(rendered.contains("Not financial advice"));
    }

    #[tokio::test]
    async fn invalid_ticker_errors() {
        let cfg = AppConfig::new("$$$".into(), false, false, false, false, 50, OutputFormat::Table);
        assert!(analyze(&cfg, &mock_social(), Some(&MockMarketSource))
            .await
            .is_err());
    }
```

- [ ] **Step 3: `main.rs` — build the social list from credentials**

Replace the `Command::Analyze` branch body in `src/main.rs` (and add imports `use openintel::adapters::sources::reddit::RedditSource;`, `use openintel::adapters::sources::mock_bluesky::MockBlueskySource;`, `use openintel::adapters::sources::mock_x::MockXSource;`, `use openintel::domain::ports::social_data_source::SocialDataSource;`). Change the `_credentials` binding to `credentials` and build the list:

```rust
        Command::Analyze(args) => {
            let config = to_app_config(&args);

            let mut social: Vec<Box<dyn SocialDataSource>> = Vec::new();
            if let (Some(id), Some(secret)) =
                (credentials.reddit_client_id, credentials.reddit_client_secret)
            {
                match RedditSource::new(id, secret) {
                    Ok(src) => social.push(Box::new(src)),
                    Err(e) => eprintln!("warning: reddit disabled: {e}"),
                }
            }
            social.push(Box::new(MockXSource));
            social.push(Box::new(MockBlueskySource));

            let outcome = if config.market_enabled {
                let market = match YahooMarketSource::new() {
                    Ok(m) => m,
                    Err(e) => {
                        eprintln!("error: {e}");
                        return ExitCode::FAILURE;
                    }
                };
                analyze(&config, &social, Some(&market)).await
            } else {
                analyze(&config, &social, None).await
            };
            match outcome {
                Ok((_report, rendered)) => {
                    println!("{rendered}");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            }
        }
```

Change the credentials line above the `match` from `let _credentials = Credentials::from_env();` to `let credentials = Credentials::from_env();` and update its comment to `// Reddit client credentials (if set) enable the real Reddit source; other sources need none.`

- [ ] **Step 4: `mcp::tools` — thread the social slice through all four functions**

In `src/mcp/tools.rs`, add `use crate::domain::ports::social_data_source::SocialDataSource;`. Change:

`run_list_sources` to report the wired social sources:

```rust
pub fn run_list_sources(
    social_sources: &[Box<dyn SocialDataSource>],
    market_source: &dyn MarketDataSource,
) -> SourcesOutput {
    SourcesOutput {
        social: social_sources
            .iter()
            .map(|s| s.kind().as_str().to_string())
            .collect(),
        market: vec![market_source.name().to_string()],
    }
}
```

`run_analyze` — accept and forward:

```rust
pub async fn run_analyze(
    args: AnalyzeArgs,
    social_sources: &[Box<dyn SocialDataSource>],
    market_source: &dyn MarketDataSource,
) -> Result<AnalyzeOutput, DomainError> {
    let req = request_from(
        args.ticker,
        args.enable_reddit,
        args.enable_x,
        args.enable_bluesky,
        args.no_market,
        args.limit,
    );
    let report = application::analyze(&req, social_sources, Some(market_source)).await?;
    Ok(AnalyzeOutput {
        summary: summarize(&report),
        report,
        disclaimer: DISCLAIMER,
    })
}
```

`run_scan` — accept the slice; each concurrent closure captures both `&[...]` and `&dyn` by copy:

```rust
pub async fn run_scan(
    args: ScanArgs,
    social_sources: &[Box<dyn SocialDataSource>],
    market_source: &dyn MarketDataSource,
) -> ScanOutput {
    let ScanArgs {
        tickers,
        enable_reddit,
        enable_x,
        enable_bluesky,
        no_market,
        limit,
    } = args;
    let futures = tickers.into_iter().map(|t| async move {
        let req = request_from(t.clone(), enable_reddit, enable_x, enable_bluesky, no_market, limit);
        match application::analyze(&req, social_sources, Some(market_source)).await {
            Ok(report) => ScanEntry {
                ticker: t,
                report: Some(report),
                error: None,
            },
            Err(e) => ScanEntry {
                ticker: t,
                report: None,
                error: Some(e.to_string()),
            },
        }
    });
    let entries = futures::future::join_all(futures).await;
    ScanOutput {
        entries,
        disclaimer: DISCLAIMER,
    }
}
```

`run_compare` — same threading:

```rust
pub async fn run_compare(
    args: CompareArgs,
    social_sources: &[Box<dyn SocialDataSource>],
    market_source: &dyn MarketDataSource,
) -> CompareOutput {
    let CompareArgs {
        tickers,
        rank_by,
        enable_reddit,
        enable_x,
        enable_bluesky,
        no_market,
        limit,
    } = args;
    let futures = tickers.into_iter().map(|t| async move {
        let req = request_from(t.clone(), enable_reddit, enable_x, enable_bluesky, no_market, limit);
        (t, application::analyze(&req, social_sources, Some(market_source)).await)
    });
    let results = futures::future::join_all(futures).await;

    let mut ranked: Vec<RankedEntry> = Vec::new();
    let mut errors: Vec<CompareError> = Vec::new();
    for (ticker, res) in results {
        match res {
            Ok(report) => {
                let metric = rank_metric(&report, rank_by);
                ranked.push(RankedEntry { ticker, rank_metric: metric, report });
            }
            Err(e) => errors.push(CompareError { ticker, error: e.to_string() }),
        }
    }
    sort_ranked(&mut ranked, rank_by);

    CompareOutput { rank_by, ranked, errors, disclaimer: DISCLAIMER }
}
```

Update the `#[cfg(test)] mod tests`: add the mock helpers and thread them. Add the imports and helper at the top of the test module, and pass `&mock_social()` + `&MockMarketSource`:

```rust
    use crate::adapters::market::mock_market::MockMarketSource;
    use crate::adapters::sources::mock_bluesky::MockBlueskySource;
    use crate::adapters::sources::mock_reddit::MockRedditSource;
    use crate::adapters::sources::mock_x::MockXSource;

    fn mock_social() -> Vec<Box<dyn SocialDataSource>> {
        vec![
            Box::new(MockRedditSource),
            Box::new(MockXSource),
            Box::new(MockBlueskySource),
        ]
    }
```

Then update each call:
- `list_sources_reports_all_adapters`: `run_list_sources(&mock_social(), &MockMarketSource)` — assertion `vec!["reddit","x","bluesky"]` and `vec!["mock-market"]` still hold (mock_social is in reddit/x/bluesky order).
- `run_analyze(args, ...)` → `run_analyze(args, &mock_social(), &MockMarketSource)` (both tests).
- `run_scan(ScanArgs {...}, ...)` → `run_scan(ScanArgs {...}, &mock_social(), &MockMarketSource)` (both tests).
- `run_compare(CompareArgs {...}, ...)` → `run_compare(CompareArgs {...}, &mock_social(), &MockMarketSource)`.
- `sort_ranked_orders_by_crowding_desc` calls no `run_*` — leave unchanged.

- [ ] **Step 5: `mcp::server` — own the social list; build it in `serve()`**

In `src/mcp/server.rs`, add imports:

```rust
use std::sync::Arc;

use crate::adapters::sources::mock_bluesky::MockBlueskySource;
use crate::adapters::sources::mock_x::MockXSource;
use crate::adapters::sources::reddit::RedditSource;
use crate::adapters::market::yahoo::YahooMarketSource;
use crate::config::secrets::Credentials;
use crate::domain::ports::social_data_source::SocialDataSource;
```

Change the struct + constructor:

```rust
#[derive(Clone)]
pub struct OpenIntelServer {
    tool_router: ToolRouter<OpenIntelServer>,
    social: Arc<Vec<Box<dyn SocialDataSource>>>,
    market: YahooMarketSource,
}

impl OpenIntelServer {
    pub fn new(social: Vec<Box<dyn SocialDataSource>>, market: YahooMarketSource) -> Self {
        Self {
            tool_router: Self::tool_router(),
            social: Arc::new(social),
            market,
        }
    }
}
```

Pass `&self.social` + `&self.market` in each tool method body:
- `list_sources`: `tools::run_list_sources(&self.social, &self.market)`
- `analyze_ticker`: `tools::run_analyze(args, &self.social, &self.market).await`
- `scan_watchlist`: `tools::run_scan(args, &self.social, &self.market).await`
- `compare_tickers`: `tools::run_compare(args, &self.social, &self.market).await`

Build both sources in `serve()`:

```rust
pub async fn serve() -> Result<(), Box<dyn std::error::Error>> {
    let credentials = Credentials::from_env();
    let mut social: Vec<Box<dyn SocialDataSource>> = Vec::new();
    if let (Some(id), Some(secret)) =
        (credentials.reddit_client_id, credentials.reddit_client_secret)
    {
        match RedditSource::new(id, secret) {
            Ok(src) => social.push(Box::new(src)),
            Err(e) => eprintln!("warning: reddit disabled: {e}"),
        }
    }
    social.push(Box::new(MockXSource));
    social.push(Box::new(MockBlueskySource));

    let market = YahooMarketSource::new()?;
    let service = OpenIntelServer::new(social, market).serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
```

- [ ] **Step 6: `tests/analyze_flow.rs` — inject the mock social list**

In `tests/analyze_flow.rs`, add imports + helper and thread through the three calls (the market-disabled one passes `None`):

```rust
use openintel::adapters::market::mock_market::MockMarketSource;
use openintel::adapters::sources::mock_bluesky::MockBlueskySource;
use openintel::adapters::sources::mock_reddit::MockRedditSource;
use openintel::adapters::sources::mock_x::MockXSource;
use openintel::domain::ports::social_data_source::SocialDataSource;

fn mock_social() -> Vec<Box<dyn SocialDataSource>> {
    vec![
        Box::new(MockRedditSource),
        Box::new(MockXSource),
        Box::new(MockBlueskySource),
    ]
}
```

- `end_to_end_all_sources_with_market`: `analyze(&cfg(false, false, false, false), &mock_social(), Some(&MockMarketSource))`
- `single_source_only`: `analyze(&cfg(true, false, false, false), &mock_social(), Some(&MockMarketSource))`
- `social_only_when_market_disabled`: `analyze(&cfg(false, false, false, true), &mock_social(), None)`

- [ ] **Step 7: Run the whole suite (green + hermetic)**

Run: `cargo test`
Expected: PASS — all prior tests plus Task 1/2/3 tests; only the Reddit + Yahoo live tests are `#[ignore]`d. No network during the run.

- [ ] **Step 8: Lint + format + build**

Run: `cargo clippy --all-targets -- -D warnings && cargo fmt --check && cargo build`
Expected: clean; binary builds.

- [ ] **Step 9: Commit**

```bash
git add src/application/analyze.rs src/cli/run.rs src/main.rs src/mcp/tools.rs src/mcp/server.rs tests/analyze_flow.rs
git commit -m "refactor: inject social sources; wire Reddit at composition roots"
```

---

## Task 5: Docs

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Update the Quickstart data-source note**

In `README.md`, replace the Quickstart caveat line (the `> **Market data is live …` block) with:

```
> **Market data is live (Yahoo Finance, keyless). Reddit is live when configured (OAuth — see below); X and Bluesky are still mock.** `analyze` fetches over the network — offline or unconfigured sources degrade gracefully with a note.
```

- [ ] **Step 2: Add a Reddit setup section**

In `README.md`, immediately after the `## Usage` flags table, add:

```
## Enable the Reddit source (optional)

Reddit requires OAuth (there is no keyless access). One-time setup:

1. Create a **script** app at <https://www.reddit.com/prefs/apps> → note the **client id** (under the app name) and **secret**.
2. Export them before running:

```bash
export OPENINTEL_REDDIT_CLIENT_ID=your_client_id
export OPENINTEL_REDDIT_CLIENT_SECRET=your_secret
openintel analyze AAPL --enable-reddit
```

Without these, `--enable-reddit` yields a `reddit enabled but not configured` note and the other sources still run. Credentials are read only from the environment, wrapped in `SecretString` (never logged or written to disk), and sent only to Reddit over TLS.
```

- [ ] **Step 3: Update the Extending note for social sources**

In the `## Extending` section, replace the "Add a social source" block:

```
**Add a social source** (e.g. real Reddit):
1. New struct in `src/adapters/sources/`, `impl SocialDataSource`.
2. Add a `SourceKind` variant in `src/domain/values/source_kind.rs`.
3. Add one arm to `build_sources` in `src/cli/run.rs`.
```

with:

```
**Add a social source** (e.g. real Bluesky):
1. New struct in `src/adapters/sources/`, `impl SocialDataSource`.
2. Add a `SourceKind` variant in `src/domain/values/source_kind.rs` if new.
3. Construct it at the composition roots — `main.rs` (analyze branch) and `mcp::server::serve()` — and push it onto the injected social list. No engine or application change.
```

- [ ] **Step 4: Update the secrets paragraph**

In the `## Architecture` section, update the secrets sentence to list the Reddit variables:

Replace `OPENINTEL_REDDIT_TOKEN` in the environment-variables sentence with `OPENINTEL_REDDIT_CLIENT_ID`, `OPENINTEL_REDDIT_CLIENT_SECRET`.

- [ ] **Step 5: Commit**

```bash
git add README.md
git commit -m "docs: Reddit OAuth setup + injected social-source extending note"
```

---

## Self-Review

**Spec coverage:**
- App-only `client_credentials` OAuth (token endpoint, Basic auth, bearer) → Task 2 `auth.rs`. ✓
- Mandatory descriptive User-Agent → Task 2 `RedditSource::new` + every request. ✓
- Env-only `SecretString` creds (`CLIENT_ID`/`CLIENT_SECRET`, replacing `REDDIT_TOKEN`) → Task 3. ✓
- Token caching + 60s-skew refresh → Task 2 `CachedToken`/`ensure_token`. ✓
- Finance-subreddit search + field mapping (id/author/text/created/score→engagement) → Task 1 `parse_posts` + Task 2 request. ✓
- Graceful absence note → Task 4 `analyze` + composition roots. ✓
- Social-side DI at composition roots, mocks in tests → Task 4. ✓
- `SourceFailure`-only errors, no `unwrap` on network data → Tasks 1/2. ✓
- Hermetic tests + one `#[ignore]` live test → Tasks 1/2 units, Task 2 live test, Task 4 Step 7. ✓
- README setup → Task 5. ✓
- Non-goals (submissions only, fixed subs, no retry/keychain) → honored. ✓

**Type consistency:** `parse_posts(body, limit, fetched_at)` produced in Task 1, consumed in Task 2. `RedditSource::new(SecretString, SecretString) -> Result` produced in Task 2, consumed in Task 4 roots. `social_sources: &[Box<dyn SocialDataSource>]` identical across `application::analyze`, `cli::run::analyze`, the four `mcp::tools` fns, and the mock `mock_social()` helper. `Credentials.reddit_client_id/reddit_client_secret` produced in Task 3, consumed in Task 4. Server field `Arc<Vec<Box<dyn SocialDataSource>>>` is `Clone`, preserving `#[derive(Clone)]`.

**Placeholder scan:** No TBD/TODO; every code step contains complete code, including the `.form()` fallback note.
