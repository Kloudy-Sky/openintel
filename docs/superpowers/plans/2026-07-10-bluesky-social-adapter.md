# Bluesky Social Adapter + All-Real Social Wiring Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A real, app-password-authenticated Bluesky sentiment source; all mocks out of production wiring; X fully excised — every wired source is real.

**Architecture:** New `src/adapters/sources/bluesky/{mod,auth,response}.rs` mirroring the Reddit adapter (pure parser, token cache, HTTP at the edge). Mocks are deleted and replaced by a `#[cfg(test)]` fixture module whose rows reproduce the old mock texts exactly (so fusion-math assertions survive). X is removed from CLI, config, domain, and MCP.

**Tech Stack:** Rust, reqwest 0.13.4 (`default-features=false, features=["rustls"]`), tokio `RwLock`, secrecy, serde, chrono, base64 0.22, clap.

**Spec:** `docs/superpowers/specs/2026-07-10-bluesky-social-adapter-design.md` — copy and endpoints in this plan are from it verbatim.

## Global Constraints

- **Env-only:** creds only via `Credentials::from_env()`; never written to disk. Empty-string env vars are unset.
- **Secrets sealed:** app password is `SecretString`; `.expose_secret()` appears in the new code at exactly ONE site (the createSession body build). The handle is public info → plain `String`.
- **reqwest 0.13.4 gating:** `.query()`, `.form()`, AND `.json()` are unavailable (features not enabled). Query strings via `reqwest::Url::query_pairs_mut()`; JSON bodies via `serde_json::to_string` + `.body(...)` + explicit `Content-Type` header. Do not add reqwest features.
- **Error substrings (setup hints key on these):** 401 → message contains `"unauthorized — check handle/app password"`; 429 → `"rate limited (HTTP 429)"`. `name: "bluesky"` on every `SourceFailure`.
- **Endpoints (atproto lexicons):** `POST https://bsky.social/xrpc/com.atproto.server.createSession` body `{"identifier":…,"password":…}` → `{"accessJwt":…}`; `GET https://bsky.social/xrpc/app.bsky.feed.searchPosts?q=<TICKER>&sort=latest&limit=<min(n,100)>` with `Authorization: Bearer`.
- **Token expiry:** decode the accessJwt's `exp` claim (base64url no-pad payload); fallback fixed TTL 10 minutes; refresh 60 s early (`SKEW_SECS: i64 = 60`).
- **Hermetic tests:** `cargo test` never touches the network; exactly one `#[ignore]`d live test.
- **stdout discipline:** no `println!` outside `src/main.rs` and `src/cli/setup.rs`.
- **No mock social sources in the library's public API** after Task 4.
- **Every commit green:** `cargo build --all-targets && cargo test && cargo clippy -- -D warnings && cargo fmt -- --check`.

---

### Task 1: Bluesky pure response parser

**Files:**
- Create: `src/adapters/sources/bluesky/mod.rs` (stub), `src/adapters/sources/bluesky/response.rs`
- Modify: `src/adapters/sources/mod.rs:4` (add module)

**Interfaces:**
- Consumes: `PostText::parse`, `SocialPost`, `SourceKind::Bluesky`, `DomainError::SourceFailure` (all existing).
- Produces: `pub(crate) fn parse_posts(body: &str, limit: usize, fetched_at: DateTime<Utc>) -> Result<Vec<SocialPost>, DomainError>` in `crate::adapters::sources::bluesky::response` — Task 2 calls it from `fetch`.

- [ ] **Step 1: Create the stub `src/adapters/sources/bluesky/mod.rs`**

```rust
// Transient: the parser lands before the HTTP source that calls it (next task
// removes this file-scoped allow when `BlueskySource` wires everything up).
#![allow(dead_code)]

mod response;
```

And in `src/adapters/sources/mod.rs`, change the module list at the top to (alphabetical):

```rust
pub mod bluesky;
pub mod mock_bluesky;
pub mod mock_reddit;
pub mod mock_x;
pub mod reddit;
```

- [ ] **Step 2: Write `src/adapters/sources/bluesky/response.rs` — tests first, then impl (single file creation)**

```rust
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::domain::entities::social_post::{PostText, SocialPost};
use crate::domain::error::DomainError;
use crate::domain::values::source_kind::SourceKind;

#[derive(Debug, Deserialize)]
struct SearchResponse {
    #[serde(default)]
    posts: Vec<PostView>,
}

#[derive(Debug, Deserialize)]
struct PostView {
    #[serde(default)]
    uri: Option<String>,
    #[serde(default)]
    author: Option<Author>,
    #[serde(default)]
    record: Option<Record>,
    #[serde(default, rename = "indexedAt")]
    indexed_at: Option<String>,
    #[serde(default, rename = "likeCount")]
    like_count: Option<i64>,
    #[serde(default, rename = "repostCount")]
    repost_count: Option<i64>,
    #[serde(default, rename = "replyCount")]
    reply_count: Option<i64>,
}

#[derive(Debug, Deserialize, Default)]
struct Author {
    #[serde(default)]
    handle: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct Record {
    #[serde(default)]
    text: Option<String>,
    #[serde(default, rename = "createdAt")]
    created_at: Option<String>,
}

fn fail(message: impl Into<String>) -> DomainError {
    DomainError::SourceFailure {
        name: "bluesky".into(),
        message: message.into(),
    }
}

fn parse_rfc3339(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

pub(crate) fn parse_posts(
    body: &str,
    limit: usize,
    fetched_at: DateTime<Utc>,
) -> Result<Vec<SocialPost>, DomainError> {
    let resp: SearchResponse =
        serde_json::from_str(body).map_err(|e| fail(format!("malformed response: {e}")))?;

    if limit == 0 {
        return Ok(Vec::new());
    }

    let mut posts = Vec::new();
    for view in resp.posts {
        let id = match view.uri {
            Some(u) if !u.is_empty() => u,
            _ => continue,
        };
        let record = view.record.unwrap_or_default();
        let text = match PostText::parse(&record.text.unwrap_or_default()) {
            Ok(t) => t,
            Err(_) => continue, // empty/whitespace text -> skip, not fatal
        };
        let created_at = record
            .created_at
            .as_deref()
            .and_then(parse_rfc3339)
            .or_else(|| view.indexed_at.as_deref().and_then(parse_rfc3339))
            .unwrap_or(fetched_at);
        let engagement = [view.like_count, view.repost_count, view.reply_count]
            .iter()
            .map(|c| c.unwrap_or(0).max(0) as u64)
            .sum::<u64>()
            .min(u32::MAX as u64) as u32;

        posts.push(SocialPost {
            id,
            source: SourceKind::Bluesky,
            author: view
                .author
                .unwrap_or_default()
                .handle
                .unwrap_or_else(|| "[unknown]".to_string()),
            text,
            created_at,
            engagement,
        });
        if posts.len() >= limit {
            break;
        }
    }
    Ok(posts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 10, 0, 0, 0).unwrap()
    }

    const HAPPY: &str = r#"{"posts":[
        {"uri":"at://did:plc:abc/app.bsky.feed.post/1","author":{"handle":"indexfan.bsky.social"},
         "record":{"text":"$AAPL calls printing","createdAt":"2026-07-09T15:30:00Z"},
         "indexedAt":"2026-07-09T15:31:00Z","likeCount":10,"repostCount":3,"replyCount":2},
        {"uri":"at://did:plc:def/app.bsky.feed.post/2","author":{"handle":"skeptic.bsky.social"},
         "record":{"text":"AAPL looks toppy, selling"},"likeCount":1}
    ]}"#;

    #[test]
    fn happy_maps_posts() {
        let posts = parse_posts(HAPPY, 50, at()).unwrap();
        assert_eq!(posts.len(), 2);
        assert_eq!(posts[0].id, "at://did:plc:abc/app.bsky.feed.post/1");
        assert_eq!(posts[0].author, "indexfan.bsky.social");
        assert_eq!(posts[0].text.as_str(), "$AAPL calls printing");
        assert_eq!(posts[0].engagement, 15); // 10 likes + 3 reposts + 2 replies
        assert_eq!(posts[0].source, SourceKind::Bluesky);
        assert_eq!(
            posts[0].created_at,
            Utc.with_ymd_and_hms(2026, 7, 9, 15, 30, 0).unwrap()
        );
    }

    #[test]
    fn missing_created_at_falls_back_to_fetched_at_and_missing_counts_are_zero() {
        let posts = parse_posts(HAPPY, 50, at()).unwrap();
        assert_eq!(posts[1].created_at, at()); // no createdAt, no indexedAt
        assert_eq!(posts[1].engagement, 1); // only likeCount present
    }

    #[test]
    fn indexed_at_is_fallback_when_created_at_missing() {
        let body = r#"{"posts":[{"uri":"u1","record":{"text":"hi"},"indexedAt":"2026-07-09T12:00:00Z"}]}"#;
        let posts = parse_posts(body, 50, at()).unwrap();
        assert_eq!(
            posts[0].created_at,
            Utc.with_ymd_and_hms(2026, 7, 9, 12, 0, 0).unwrap()
        );
        assert_eq!(posts[0].author, "[unknown]");
    }

    #[test]
    fn empty_text_and_missing_uri_are_skipped() {
        let body = r#"{"posts":[
            {"uri":"u1","record":{"text":"   "}},
            {"record":{"text":"no uri"}},
            {"uri":"u2","record":{"text":"kept"}}
        ]}"#;
        let posts = parse_posts(body, 50, at()).unwrap();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].text.as_str(), "kept");
    }

    #[test]
    fn limit_truncates_and_zero_is_empty() {
        assert_eq!(parse_posts(HAPPY, 1, at()).unwrap().len(), 1);
        assert!(parse_posts(HAPPY, 0, at()).unwrap().is_empty());
    }

    #[test]
    fn engagement_saturates_at_u32_max() {
        let body = r#"{"posts":[{"uri":"u1","record":{"text":"big"},
            "likeCount":4294967295,"repostCount":4294967295,"replyCount":10}]}"#;
        let posts = parse_posts(body, 50, at()).unwrap();
        assert_eq!(posts[0].engagement, u32::MAX);
    }

    #[test]
    fn malformed_json_is_failure_and_empty_posts_ok() {
        assert!(parse_posts("nope", 50, at()).is_err());
        assert!(parse_posts(r#"{"posts":[]}"#, 50, at()).unwrap().is_empty());
    }
}
```

- [ ] **Step 3: Run the parser tests**

Run: `cargo test --lib adapters::sources::bluesky`
Expected: PASS (7 tests).

- [ ] **Step 4: Full verification**

Run: `cargo build --all-targets && cargo test && cargo clippy -- -D warnings && cargo fmt`
Expected: all green (fmt normalizes, then `cargo fmt -- --check` clean).

- [ ] **Step 5: Commit**

```bash
git add src/adapters/sources/bluesky/ src/adapters/sources/mod.rs
git commit -m "feat(bluesky): pure searchPosts response parser

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 2: Bluesky auth + HTTP source

**Files:**
- Create: `src/adapters/sources/bluesky/auth.rs`
- Modify: `src/adapters/sources/bluesky/mod.rs` (full implementation, remove transient allow), `Cargo.toml` (add `base64 = "0.22"`)

**Interfaces:**
- Consumes: Task 1's `response::parse_posts`; existing `SocialDataSource` trait, `Ticker`, `DomainError`.
- Produces: `pub struct BlueskySource` with `pub fn new(handle: String, app_password: SecretString) -> Result<Self, DomainError>` and `impl SocialDataSource` (`kind() -> SourceKind::Bluesky`, `fetch`). Tasks 5 and 6 construct it.

- [ ] **Step 1: Add the dependency**

Run: `cargo add base64@0.22`
Expected: resolves to 0.22.1 already in Cargo.lock (no new packages).

- [ ] **Step 2: Write `src/adapters/sources/bluesky/auth.rs`**

```rust
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Duration, TimeZone, Utc};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

use crate::domain::error::DomainError;

const SKEW_SECS: i64 = 60;
const FALLBACK_TTL_SECS: i64 = 600; // accessJwt with an undecodable exp: assume 10 minutes
const SESSION_URL: &str = "https://bsky.social/xrpc/com.atproto.server.createSession";

pub(crate) struct CachedToken {
    pub bearer: SecretString,
    pub expires_at: DateTime<Utc>,
}

impl CachedToken {
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        now + Duration::seconds(SKEW_SECS) >= self.expires_at
    }
}

#[derive(Serialize)]
struct SessionRequest<'a> {
    identifier: &'a str,
    password: &'a str,
}

#[derive(Deserialize)]
struct SessionResponse {
    #[serde(default, rename = "accessJwt")]
    access_jwt: Option<String>,
}

fn fail(message: impl Into<String>) -> DomainError {
    DomainError::SourceFailure {
        name: "bluesky".into(),
        message: message.into(),
    }
}

/// Decode the `exp` claim (unix seconds) from a JWT without verifying it —
/// we only need a refresh hint, not trust (the server enforces real expiry).
pub(crate) fn parse_jwt_exp(jwt: &str) -> Option<DateTime<Utc>> {
    let payload = jwt.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let exp = value.get("exp")?.as_i64()?;
    Utc.timestamp_opt(exp, 0).single()
}

pub(crate) fn parse_session(body: &str, now: DateTime<Utc>) -> Result<CachedToken, DomainError> {
    let resp: SessionResponse =
        serde_json::from_str(body).map_err(|e| fail(format!("malformed session response: {e}")))?;
    let access_jwt = resp
        .access_jwt
        .filter(|t| !t.is_empty())
        .ok_or_else(|| fail("no accessJwt in response"))?;
    let expires_at = parse_jwt_exp(&access_jwt)
        .unwrap_or_else(|| now + Duration::seconds(FALLBACK_TTL_SECS));
    Ok(CachedToken {
        bearer: SecretString::new(access_jwt.into_boxed_str()),
        expires_at,
    })
}

pub(crate) async fn request_session(
    client: &reqwest::Client,
    handle: &str,
    app_password: &SecretString,
    user_agent: &str,
    now: DateTime<Utc>,
) -> Result<CachedToken, DomainError> {
    // reqwest's `.json()` is behind the un-enabled `json` feature; build the body manually.
    let body = serde_json::to_string(&SessionRequest {
        identifier: handle,
        password: app_password.expose_secret(),
    })
    .map_err(|e| fail(format!("session body build failed: {e}")))?;
    let resp = client
        .post(SESSION_URL)
        .header(reqwest::header::USER_AGENT, user_agent)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await
        .map_err(|e| fail(format!("session request failed: {e}")))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| fail(format!("session body failed (HTTP {status}): {e}")))?;
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(fail("unauthorized — check handle/app password"));
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(fail("rate limited (HTTP 429)"));
    }
    if !status.is_success() {
        return Err(fail(format!("session request HTTP {status}")));
    }
    parse_session(&text, now)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 10, 0, 0, 0).unwrap()
    }

    /// Build an unsigned JWT with the given JSON payload (header/signature irrelevant to exp parsing).
    fn jwt_with_payload(payload: &str) -> String {
        let enc = |s: &str| URL_SAFE_NO_PAD.encode(s.as_bytes());
        format!("{}.{}.{}", enc(r#"{"alg":"none"}"#), enc(payload), enc("sig"))
    }

    #[test]
    fn parse_jwt_exp_reads_exp_claim() {
        // 2026-07-10T02:00:00Z = 1783648800
        let jwt = jwt_with_payload(r#"{"scope":"com.atproto.appPass","exp":1783648800}"#);
        assert_eq!(
            parse_jwt_exp(&jwt).unwrap(),
            Utc.with_ymd_and_hms(2026, 7, 10, 2, 0, 0).unwrap()
        );
    }

    #[test]
    fn parse_jwt_exp_handles_garbage() {
        assert!(parse_jwt_exp("not-a-jwt").is_none());
        assert!(parse_jwt_exp("a.b.c").is_none()); // b is not valid base64 JSON
        let no_exp = jwt_with_payload(r#"{"scope":"x"}"#);
        assert!(parse_jwt_exp(&no_exp).is_none());
    }

    #[test]
    fn parse_session_uses_exp_and_skew() {
        let jwt = jwt_with_payload(r#"{"exp":1783648800}"#); // 2026-07-10T02:00:00Z
        let body = format!(r#"{{"accessJwt":"{jwt}","refreshJwt":"r","handle":"h","did":"d"}}"#);
        let t = parse_session(&body, at()).unwrap();
        assert!(!t.is_expired(at()));
        // 60s skew: expired from 01:59:00 onward
        assert!(t.is_expired(Utc.with_ymd_and_hms(2026, 7, 10, 1, 59, 0).unwrap()));
        assert!(!t.is_expired(Utc.with_ymd_and_hms(2026, 7, 10, 1, 58, 59).unwrap()));
    }

    #[test]
    fn parse_session_undecodable_exp_falls_back_to_ttl() {
        let body = r#"{"accessJwt":"opaque-token","refreshJwt":"r","handle":"h","did":"d"}"#;
        let t = parse_session(body, at()).unwrap();
        assert_eq!(t.expires_at, at() + Duration::seconds(600));
    }

    #[test]
    fn parse_session_missing_or_malformed_is_failure() {
        assert!(parse_session(r#"{"handle":"h"}"#, at()).is_err());
        assert!(parse_session("nope", at()).is_err());
    }
}
```

- [ ] **Step 3: Run auth tests**

Run: `cargo test --lib adapters::sources::bluesky::auth`
Expected: PASS (5 tests).

- [ ] **Step 4: Replace `src/adapters/sources/bluesky/mod.rs` with the full source**

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
        let user_agent = format!(
            "rust:openintel:v{} (by /u/openintel)",
            env!("CARGO_PKG_VERSION")
        );
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
        let src =
            BlueskySource::new(handle, SecretString::new(pw.into_boxed_str())).unwrap();
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
}
```

(Note this file no longer has the `#![allow(dead_code)]` — everything is now used.)

- [ ] **Step 5: Full verification**

Run: `cargo build --all-targets && cargo test && cargo clippy -- -D warnings && cargo fmt`
Expected: all green; bluesky tests = 7 parser + 5 auth + 1 unit + 1 ignored.

- [ ] **Step 6: Commit**

```bash
git add src/adapters/sources/bluesky/ Cargo.toml Cargo.lock
git commit -m "feat(bluesky): app-password OAuth source — createSession + authed searchPosts

JWT-exp token cache (base64url decode, 10-min fallback TTL, 60s skew,
RwLock single-flight). Manual JSON body (reqwest json feature not enabled).

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 3: Credentials — add `bluesky_handle`, remove `x_bearer`

**Files:**
- Modify: `src/config/secrets.rs`

**Interfaces:**
- Produces: `Credentials { reddit_client_id, reddit_client_secret, bluesky_handle: Option<String>, bluesky_app_password, market_api_key }`; env var `OPENINTEL_BLUESKY_HANDLE`. Tasks 4–6 consume this exact shape. **`x_bearer` no longer exists** — any other file referencing it must be updated in the same commit (only `secrets.rs` references it today).

- [ ] **Step 1: Replace the struct and `from_env`**

```rust
use secrecy::SecretString;

#[derive(Debug)]
pub struct Credentials {
    pub reddit_client_id: Option<SecretString>,
    pub reddit_client_secret: Option<SecretString>,
    /// Bluesky handle (e.g. `name.bsky.social`) — public info, so a plain String.
    pub bluesky_handle: Option<String>,
    pub bluesky_app_password: Option<SecretString>,
    pub market_api_key: Option<SecretString>,
}

impl Credentials {
    pub fn from_env() -> Self {
        Credentials {
            reddit_client_id: secret_from(std::env::var("OPENINTEL_REDDIT_CLIENT_ID").ok()),
            reddit_client_secret: secret_from(std::env::var("OPENINTEL_REDDIT_CLIENT_SECRET").ok()),
            bluesky_handle: plain_from(std::env::var("OPENINTEL_BLUESKY_HANDLE").ok()),
            bluesky_app_password: secret_from(
                std::env::var("OPENINTEL_BLUESKY_APP_PASSWORD").ok(),
            ),
            market_api_key: secret_from(std::env::var("OPENINTEL_MARKET_API_KEY").ok()),
        }
    }
}

fn secret_from(value: Option<String>) -> Option<SecretString> {
    value
        .filter(|v| !v.is_empty())
        .map(|v| SecretString::new(v.into_boxed_str()))
}

/// Non-secret env value with the same "exported-but-empty means unset" rule.
fn plain_from(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.is_empty())
}
```

- [ ] **Step 2: Update the tests in the same file**

In `wraps_present_value_and_skips_absent`, append:

```rust
        assert_eq!(plain_from(Some("me.bsky.social".into())).as_deref(), Some("me.bsky.social"));
        assert!(plain_from(Some(String::new())).is_none());
        assert!(plain_from(None).is_none());
```

In `debug_does_not_leak_secret`, replace the struct literal with:

```rust
        let creds = Credentials {
            reddit_client_id: secret_from(Some("leak-me".to_string())),
            reddit_client_secret: None,
            bluesky_handle: Some("public.bsky.social".into()),
            bluesky_app_password: secret_from(Some("leak-me-too".to_string())),
            market_api_key: None,
        };
        assert!(!format!("{creds:?}").contains("leak-me"));
        assert!(!format!("{creds:?}").contains("leak-me-too"));
```

(Note: `!contains("leak-me")` implies `!contains("leak-me-too")` fails wrongly — `"leak-me-too"` CONTAINS `"leak-me"` as a substring, so the first assert alone covers both; keep both asserts anyway for clarity, they are consistent: if neither string appears, both pass.)

Also update `src/adapters/sources/mod.rs`'s test helper `creds(...)` (it constructs `Credentials` with `x_bearer`): replace the `x_bearer: None,` line with `bluesky_handle: None,` (keep `bluesky_app_password: None`).

- [ ] **Step 3: Verify + commit**

Run: `cargo build --all-targets && cargo test && cargo clippy -- -D warnings && cargo fmt`
Expected: green.

```bash
git add src/config/secrets.rs src/adapters/sources/mod.rs
git commit -m "feat(config): bluesky handle credential; drop unused x_bearer

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 4: X excision + mocks → shared test fixtures

**Files:**
- Delete: `src/adapters/sources/mock_x.rs`, `src/adapters/sources/mock_bluesky.rs`, `src/adapters/sources/mock_reddit.rs`
- Create: `src/adapters/sources/test_fixtures.rs`
- Modify: `src/adapters/sources/mod.rs`, `src/domain/values/source_kind.rs`, `src/config/settings.rs`, `src/cli/args.rs`, `src/mcp/tools.rs`, `src/application/analyze.rs` (tests), `src/cli/run.rs` (tests), `tests/analyze_flow.rs`

**Interfaces:**
- Consumes: nothing from Tasks 1–3 beyond the Credentials shape.
- Produces: `SourceKind { Reddit, Bluesky }` with `ALL: [SourceKind; 2]`; `AppConfig::new(ticker, reddit: bool, bluesky: bool, no_market: bool, limit, format)`; `#[cfg(test)] pub(crate) mod test_fixtures` exposing `FixtureSource`, `reddit_fixture()`, `bluesky_fixture()`, `fixture_social()`. After this task `build_social_sources` wires **reddit only** (Task 5 adds bluesky).

**Load-bearing invariant:** the fixture rows below are the old mocks' texts VERBATIM (Reddit's 4 rows; Bluesky's 3 rows plus the old X mock's 3 rows re-homed to Bluesky). Total = 10 posts with identical analyzer inputs, so every existing fusion assertion (`total_mentions == 10`, `ConfirmingBullish`, reddit-only `== 4`, absent-source `== 6`) keeps passing with only source-count bookkeeping updates.

- [ ] **Step 1: Create `src/adapters/sources/test_fixtures.rs`**

```rust
//! Deterministic in-memory social sources for tests (replaces the deleted
//! mock adapters — production wiring has no fakes; see the 2026-07-10 spec).

use async_trait::async_trait;
use chrono::{TimeZone, Utc};

use crate::domain::entities::social_post::{PostText, SocialPost};
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::ports::social_data_source::SocialDataSource;
use crate::domain::values::source_kind::SourceKind;

type Row = (&'static str, &'static str, &'static str, u32);

pub(crate) struct FixtureSource {
    pub kind: SourceKind,
    pub rows: &'static [Row],
}

#[async_trait]
impl SocialDataSource for FixtureSource {
    fn kind(&self) -> SourceKind {
        self.kind
    }

    async fn fetch(&self, ticker: &Ticker, limit: usize) -> Result<Vec<SocialPost>, DomainError> {
        let sym = ticker.as_str();
        self.rows
            .iter()
            .take(limit)
            .map(|(id, author, template, engagement)| {
                Ok(SocialPost {
                    id: (*id).to_string(),
                    source: self.kind,
                    author: (*author).to_string(),
                    text: PostText::parse(&template.replace("{sym}", sym))?,
                    created_at: Utc.with_ymd_and_hms(2026, 6, 24, 15, 0, 0).unwrap(),
                    engagement: *engagement,
                })
            })
            .collect()
    }
}

/// The old MockRedditSource rows, verbatim (4 posts).
pub(crate) fn reddit_fixture() -> FixtureSource {
    FixtureSource {
        kind: SourceKind::Reddit,
        rows: &[
            ("reddit-1", "dudebro", "{sym} to the moon, loading calls all day", 420),
            ("reddit-2", "valuepicker", "{sym} earnings look strong, going long here", 88),
            ("reddit-3", "chartwatcher", "{sym} breakout confirmed, rocket time", 51),
            ("reddit-4", "shortking", "{sym} is going to dump, buying puts", 31),
        ],
    }
}

/// The old MockBlueskySource rows plus the old MockXSource rows re-homed to
/// Bluesky (6 posts) — keeps the all-fixtures total at 10 (= min_sample) with
/// identical analyzer inputs, so fusion assertions are unchanged.
pub(crate) fn bluesky_fixture() -> FixtureSource {
    FixtureSource {
        kind: SourceKind::Bluesky,
        rows: &[
            ("bsky-1", "indexfan", "{sym} looking bullish into the print", 22),
            ("bsky-2", "skeptic", "not sold on {sym}, might sell my shares", 9),
            ("bsky-3", "daytripper", "{sym} green day, up big", 14),
            ("bsky-4", "quanttrader", "${sym} squeeze incoming, buying calls", 1200),
            ("bsky-5", "macroowl", "watching ${sym} but staying cautious", 64),
            ("bsky-6", "trendrider", "${sym} rally looks strong", 240),
        ],
    }
}

pub(crate) fn fixture_social() -> Vec<Box<dyn SocialDataSource>> {
    vec![Box::new(reddit_fixture()), Box::new(bluesky_fixture())]
}
```

- [ ] **Step 2: Rewrite `src/adapters/sources/mod.rs` module list + `build_social_sources` (reddit-only for now)**

Module list becomes:

```rust
pub mod bluesky;
pub mod reddit;

#[cfg(test)]
pub(crate) mod test_fixtures;
```

Delete the three mock files:

```bash
git rm src/adapters/sources/mock_x.rs src/adapters/sources/mock_bluesky.rs src/adapters/sources/mock_reddit.rs
```

In `build_social_sources`, delete the two mock pushes at the end (`social.push(Box::new(mock_x::MockXSource));` and `social.push(Box::new(mock_bluesky::MockBlueskySource));`) and update its doc comment to: "Assemble the social data sources from credentials: the real `RedditSource` when both OAuth credentials are set. A partial config or constructor failure logs a warning to stderr and omits the source. Shared by both composition roots (`main.rs` and `mcp::server::serve`)." Update its tests:

```rust
    #[test]
    fn no_creds_wires_no_sources() {
        assert!(build_social_sources(&creds(false)).is_empty());
    }

    #[test]
    fn includes_reddit_with_creds() {
        let kinds: Vec<_> = build_social_sources(&creds(true))
            .iter()
            .map(|s| s.kind())
            .collect();
        assert_eq!(kinds, vec![SourceKind::Reddit]);
    }

    #[test]
    fn partial_creds_omits_reddit() {
        let mut c = creds(true);
        c.reddit_client_secret = None; // only the client id is set
        assert!(build_social_sources(&c).is_empty());
    }
```

(`omits_reddit_without_creds` is renamed/absorbed by `no_creds_wires_no_sources`.)

- [ ] **Step 3: Excise X from the domain**

`src/domain/values/source_kind.rs`: remove the `X` variant; `ALL` becomes `pub const ALL: [SourceKind; 2] = [SourceKind::Reddit, SourceKind::Bluesky];`; `as_str` loses the `X` arm. Tests: drop the two `"x"` assertions; `all_lists_every_variant_in_order` asserts `[SourceKind::Reddit, SourceKind::Bluesky]`.

- [ ] **Step 4: Excise X from config + CLI args**

`src/config/settings.rs` — `AppConfig::new` signature becomes:

```rust
    pub fn new(
        ticker: String,
        reddit: bool,
        bluesky: bool,
        no_market: bool,
        limit: usize,
        format: OutputFormat,
    ) -> Self {
```

(delete the `if x { … }` block). Tests: `no_flags_enables_all_sources_and_market` drops one `false` arg and asserts `vec![SourceKind::Reddit, SourceKind::Bluesky]`; `single_flag_narrows_sources` drops one `false` arg.

`src/cli/args.rs`: delete the `#[arg(long)] pub enable_x: bool,` field; `to_app_config` drops `args.enable_x`. Test `maps_no_flags_to_all_sources`: `assert_eq!(cfg.enabled_sources.len(), 2);`. Add one negative test:

```rust
    #[test]
    fn enable_x_flag_no_longer_exists() {
        assert!(Cli::try_parse_from(["openintel", "analyze", "AAPL", "--enable-x"]).is_err());
    }
```

- [ ] **Step 5: Excise X from MCP tools + move tests to fixtures**

`src/mcp/tools.rs`: delete every `enable_x` field (structs at ~lines 41, 125, 208), the `enable_x: Option<bool>` parameter + `if enable_x.unwrap_or(false) { enabled.push(SourceKind::X); }` block in the `enabled_from` helper (~lines 60–70), and every `enable_x` argument at call/test sites (~lines 107, 156, 165, 272, 281, and all test literals). In the test module, replace the three mock imports + `mock_social()` with:

```rust
    use crate::adapters::sources::test_fixtures::fixture_social;
```

(and call `fixture_social()` where `mock_social()` was). `list_sources_reports_all_adapters` asserts `vec!["reddit", "bluesky"]`. The `total_mentions == 10` and `ConfirmingBullish` assertions are unchanged by design.

- [ ] **Step 6: Move `src/application/analyze.rs` and `src/cli/run.rs` tests to fixtures**

Both test modules: replace the three mock imports + local `mock_social()` with `use crate::adapters::sources::test_fixtures::fixture_social;` (analyze.rs keeps `MockMarketSource`). `src/cli/run.rs` `config()` helper drops one `false` (new `AppConfig::new` arity). In analyze.rs `enabled_source_absent_is_noted`, replace the two-mock vec with:

```rust
        // reddit enabled but not wired -> note + the bluesky fixture still counted (6 posts)
        let social: Vec<Box<dyn SocialDataSource>> =
            vec![Box::new(crate::adapters::sources::test_fixtures::bluesky_fixture())];
```

(assertions `total_mentions == 6` and the note check are unchanged).

- [ ] **Step 7: Rewrite `tests/analyze_flow.rs` with a local double**

Integration tests can't see `#[cfg(test)]` lib modules — define the double locally with the same rows:

```rust
use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use openintel::adapters::market::mock_market::MockMarketSource;
use openintel::cli::run::analyze;
use openintel::config::settings::{AppConfig, OutputFormat};
use openintel::domain::entities::social_post::{PostText, SocialPost};
use openintel::domain::entities::ticker::Ticker;
use openintel::domain::error::DomainError;
use openintel::domain::ports::social_data_source::SocialDataSource;
use openintel::domain::values::source_kind::SourceKind;
use openintel::domain::values::speculation::Alignment;

struct FixtureSource {
    kind: SourceKind,
    rows: &'static [(&'static str, &'static str, &'static str, u32)],
}

#[async_trait]
impl SocialDataSource for FixtureSource {
    fn kind(&self) -> SourceKind {
        self.kind
    }
    async fn fetch(&self, ticker: &Ticker, limit: usize) -> Result<Vec<SocialPost>, DomainError> {
        let sym = ticker.as_str();
        self.rows
            .iter()
            .take(limit)
            .map(|(id, author, template, engagement)| {
                Ok(SocialPost {
                    id: (*id).to_string(),
                    source: self.kind,
                    author: (*author).to_string(),
                    text: PostText::parse(&template.replace("{sym}", sym))?,
                    created_at: Utc.with_ymd_and_hms(2026, 6, 24, 15, 0, 0).unwrap(),
                    engagement: *engagement,
                })
            })
            .collect()
    }
}

fn cfg(reddit: bool, bluesky: bool, no_market: bool) -> AppConfig {
    AppConfig::new("AAPL".into(), reddit, bluesky, no_market, 50, OutputFormat::Json)
}

fn fixture_social() -> Vec<Box<dyn SocialDataSource>> {
    vec![
        Box::new(FixtureSource {
            kind: SourceKind::Reddit,
            rows: &[
                ("reddit-1", "dudebro", "{sym} to the moon, loading calls all day", 420),
                ("reddit-2", "valuepicker", "{sym} earnings look strong, going long here", 88),
                ("reddit-3", "chartwatcher", "{sym} breakout confirmed, rocket time", 51),
                ("reddit-4", "shortking", "{sym} is going to dump, buying puts", 31),
            ],
        }),
        Box::new(FixtureSource {
            kind: SourceKind::Bluesky,
            rows: &[
                ("bsky-1", "indexfan", "{sym} looking bullish into the print", 22),
                ("bsky-2", "skeptic", "not sold on {sym}, might sell my shares", 9),
                ("bsky-3", "daytripper", "{sym} green day, up big", 14),
                ("bsky-4", "quanttrader", "${sym} squeeze incoming, buying calls", 1200),
                ("bsky-5", "macroowl", "watching ${sym} but staying cautious", 64),
                ("bsky-6", "trendrider", "${sym} rally looks strong", 240),
            ],
        }),
    ]
}

#[tokio::test]
async fn end_to_end_all_sources_with_market() {
    let (report, json) = analyze(&cfg(false, false, false), &fixture_social(), Some(&MockMarketSource))
        .await
        .unwrap();
    // 4 reddit + 6 bluesky fixture posts (>= min_sample of 10)
    assert_eq!(report.social.total_mentions, 10);
    assert_eq!(report.fusion.alignment, Alignment::ConfirmingBullish);
    assert!(report.market.is_some());
    assert!(json.contains("\"alignment\": \"confirming_bullish\""));
    assert!(json.contains("Not financial advice"));
}

#[tokio::test]
async fn single_source_only() {
    let (report, _) = analyze(&cfg(true, false, false), &fixture_social(), Some(&MockMarketSource))
        .await
        .unwrap();
    assert_eq!(report.social.total_mentions, 4); // reddit fixtures only
}

#[tokio::test]
async fn social_only_when_market_disabled() {
    let (report, _) = analyze(&cfg(false, false, true), &fixture_social(), None)
        .await
        .unwrap();
    assert!(report.market.is_none());
    assert_eq!(report.fusion.alignment, Alignment::Quiet);
}
```

- [ ] **Step 8: Full verification**

Run: `cargo build --all-targets && cargo test && cargo clippy -- -D warnings && cargo fmt`
Expected: everything green; no `SourceKind::X`, `enable_x`, `x_bearer`, or `Mock*Source` (social) anywhere: verify with `grep -rn "SourceKind::X\|enable_x\|x_bearer\|MockXSource\|MockBlueskySource\|MockRedditSource" src/ tests/` → no matches.

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "refactor(social): excise X and delete all mock sources

X (flag, SourceKind, cred, mock) is gone — the API is paid-only and a
recognized-but-unavailable stub is permanent slop. Mock reddit/bluesky
leave the public API; tests use a cfg(test) fixture module whose rows are
the old mock texts verbatim (fusion assertions unchanged, total still 10).
build_social_sources wires reddit only until the bluesky arm lands.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 5: Wire Bluesky into production + NoData edge test

**Files:**
- Modify: `src/adapters/sources/mod.rs` (bluesky arm + tests), `src/application/analyze.rs` (one new test)

**Interfaces:**
- Consumes: `BlueskySource::new(handle: String, app_password: SecretString)` (Task 2); `Credentials.bluesky_handle`/`bluesky_app_password` (Task 3).
- Produces: `build_social_sources` wiring both real sources — the composition roots (`main.rs`, `mcp::server::serve`) need no changes (they already call it).

- [ ] **Step 1: Add the bluesky arm to `build_social_sources`** (after the reddit `match`, before `social` is returned):

```rust
    match (
        credentials.bluesky_handle.clone(),
        credentials.bluesky_app_password.clone(),
    ) {
        (Some(handle), Some(password)) => match bluesky::BlueskySource::new(handle, password) {
            Ok(src) => social.push(Box::new(src)),
            Err(e) => eprintln!("warning: bluesky disabled: {e}"),
        },
        (Some(_), None) | (None, Some(_)) => eprintln!(
            "warning: bluesky disabled: set BOTH OPENINTEL_BLUESKY_HANDLE and OPENINTEL_BLUESKY_APP_PASSWORD"
        ),
        (None, None) => {}
    }
```

Update the fn doc comment to name both real sources.

- [ ] **Step 2: Extend the gating tests in the same file**

Extend the `creds` helper with a `bluesky: bool` parameter:

```rust
    fn creds(reddit: bool, bluesky: bool) -> Credentials {
        let s = |v: &str| Some(SecretString::new(v.to_string().into_boxed_str()));
        Credentials {
            reddit_client_id: if reddit { s("id") } else { None },
            reddit_client_secret: if reddit { s("secret") } else { None },
            bluesky_handle: if bluesky { Some("me.bsky.social".into()) } else { None },
            bluesky_app_password: if bluesky { s("pw") } else { None },
            market_api_key: None,
        }
    }
```

Update existing tests to the two-arg helper (`creds(false, false)`, `creds(true, false)`), and add:

```rust
    #[test]
    fn includes_bluesky_with_creds() {
        let kinds: Vec<_> = build_social_sources(&creds(false, true))
            .iter()
            .map(|s| s.kind())
            .collect();
        assert_eq!(kinds, vec![SourceKind::Bluesky]);
    }

    #[test]
    fn includes_both_with_all_creds() {
        let kinds: Vec<_> = build_social_sources(&creds(true, true))
            .iter()
            .map(|s| s.kind())
            .collect();
        assert_eq!(kinds, vec![SourceKind::Reddit, SourceKind::Bluesky]);
    }

    #[test]
    fn partial_bluesky_creds_omits_bluesky() {
        let mut c = creds(false, true);
        c.bluesky_app_password = None; // only the handle is set
        assert!(build_social_sources(&c).is_empty());
    }
```

- [ ] **Step 3: The NoData edge test** — in `src/application/analyze.rs`'s test module:

```rust
    #[tokio::test]
    async fn zero_sources_and_no_market_is_no_data() {
        // The spec's explicit edge decision: nothing configured + --no-market
        // -> DomainError::NoData (mocks used to mask this path).
        let social: Vec<Box<dyn SocialDataSource>> = vec![];
        let err = analyze(&req("AAPL", false), &social, None).await.unwrap_err();
        assert!(matches!(err, DomainError::NoData));
    }
```

(Add `use crate::domain::error::DomainError;` to the test module if not already imported.)

- [ ] **Step 4: Verify + commit**

Run: `cargo build --all-targets && cargo test && cargo clippy -- -D warnings && cargo fmt`
Expected: green.

```bash
git add src/adapters/sources/mod.rs src/application/analyze.rs
git commit -m "feat(bluesky): wire BlueskySource into production; pin NoData edge

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 6: `openintel setup bluesky`

**Files:**
- Modify: `src/cli/args.rs` (SetupSource variant + test), `src/cli/setup.rs`

**Interfaces:**
- Consumes: `BlueskySource::new` (Task 2), `Credentials.bluesky_handle`/`bluesky_app_password` (Task 3), existing setup plumbing.
- Produces: `SetupSource::Bluesky`; shared helpers re-signed as `partial_text(source_label: &str, missing: &str)`, `verify_ok_text(source_label: &str, count: usize, try_cmd: &str)`, `verify_err_text(err: &DomainError, unauthorized_hint: &str)`; `Mode::{Verify, MissingFirst, MissingSecond, Guide}`.

- [ ] **Step 1: `src/cli/args.rs`** — add the variant:

```rust
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupSource {
    Reddit,
    Bluesky,
}
```

and a parse test:

```rust
    #[test]
    fn parses_setup_bluesky() {
        let cli = Cli::try_parse_from(["openintel", "setup", "bluesky"]).unwrap();
        let Command::Setup(args) = cli.command else {
            panic!("expected setup command");
        };
        assert_eq!(args.source, SetupSource::Bluesky);
    }
```

- [ ] **Step 2: Generalize the shared helpers in `src/cli/setup.rs`**

Rename the Mode variants (mode selection is source-agnostic):

```rust
/// Which of the three setup modes applies, given which env vars are set.
/// First/second = the source's (identifier-like, secret-like) credential pair
/// — (client id, client secret) for Reddit; (handle, app password) for Bluesky.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Verify,
    MissingFirst,
    MissingSecond,
    Guide,
}

fn plan(first_set: bool, second_set: bool) -> Mode {
    match (first_set, second_set) {
        (true, true) => Mode::Verify,
        (false, true) => Mode::MissingFirst,
        (true, false) => Mode::MissingSecond,
        (false, false) => Mode::Guide,
    }
}
```

Re-sign the shared render helpers (reddit call sites updated to match):

```rust
fn partial_text(source_label: &str, missing: &str) -> String {
    format!(
        "⚠  {source_label} is half-configured: {missing} is not set.\n   \
         Set it, then re-run. (Run `openintel setup {}` with neither variable\n   \
         set to see the full setup guide.)",
        source_label.to_lowercase()
    )
}

fn verify_ok_text(source_label: &str, count: usize, try_cmd: &str) -> String {
    let evidence = if count > 0 {
        format!("pulled {count} recent post(s) for a test query")
    } else {
        "credentials work — the test query just had no recent posts, which is fine".to_string()
    };
    format!(
        "✅ {source_label} is configured and working ({evidence}).\n   \
         Real {source_label} sentiment is active. Try:  {try_cmd}"
    )
}

fn verify_err_text(err: &DomainError, unauthorized_hint: &str) -> String {
    let msg = err.to_string();
    let hint = if msg.contains("unauthorized") {
        unauthorized_hint
    } else if msg.contains("rate limited") {
        "You're being rate-limited right now — wait a minute and re-run."
    } else {
        "Check your internet connection and try again."
    };
    format!("❌ {msg}\n   {hint}")
}

const REDDIT_UNAUTHORIZED_HINT: &str = "Your client id or secret looks wrong. Re-copy both from\n   \
         https://www.reddit.com/prefs/apps (the id is the short string under the app\n   \
         name; the secret is labelled \"secret\").";

const BLUESKY_UNAUTHORIZED_HINT: &str = "Your handle or app password looks wrong. Check the handle\n   \
         (e.g. yourname.bsky.social) and generate a fresh app password at\n   \
         https://bsky.app/settings/app-passwords (the value is shown only once).";
```

Reddit call sites become: `partial_text("Reddit", "OPENINTEL_REDDIT_CLIENT_ID")` (and `…_SECRET`), `verify_ok_text("Reddit", count, "openintel analyze GME --enable-reddit")`, `verify_err_text(&e, REDDIT_UNAUTHORIZED_HINT)`; the "Checking your Reddit credentials…" println stays in `setup_reddit`.

- [ ] **Step 3: Add the bluesky arm + guide**

```rust
pub async fn run(source: SetupSource, credentials: &Credentials) -> ExitCode {
    match source {
        SetupSource::Reddit => setup_reddit(credentials).await,
        SetupSource::Bluesky => setup_bluesky(credentials).await,
    }
}
```

```rust
async fn setup_bluesky(credentials: &Credentials) -> ExitCode {
    match plan(
        credentials.bluesky_handle.is_some(),
        credentials.bluesky_app_password.is_some(),
    ) {
        Mode::Guide => {
            println!("{}", bluesky_guide_text());
            ExitCode::FAILURE
        }
        Mode::MissingFirst => {
            println!("{}", partial_text("Bluesky", "OPENINTEL_BLUESKY_HANDLE"));
            ExitCode::FAILURE
        }
        Mode::MissingSecond => {
            println!("{}", partial_text("Bluesky", "OPENINTEL_BLUESKY_APP_PASSWORD"));
            ExitCode::FAILURE
        }
        Mode::Verify => {
            println!("Checking your Bluesky credentials…");
            let (Some(handle), Some(password)) = (
                credentials.bluesky_handle.clone(),
                credentials.bluesky_app_password.clone(),
            ) else {
                // Unreachable: Mode::Verify is returned only when both are set.
                println!("internal error: credentials unavailable");
                return ExitCode::FAILURE;
            };
            match probe_bluesky(handle, password).await {
                Ok(count) => {
                    println!(
                        "{}",
                        verify_ok_text("Bluesky", count, "openintel analyze GME --enable-bluesky")
                    );
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    println!("{}", verify_err_text(&e, BLUESKY_UNAUTHORIZED_HINT));
                    ExitCode::FAILURE
                }
            }
        }
    }
}

/// One live round trip through the full Bluesky path: createSession plus a search.
async fn probe_bluesky(handle: String, password: SecretString) -> Result<usize, DomainError> {
    let source = BlueskySource::new(handle, password)?;
    let ticker = Ticker::parse("AAPL")?;
    let posts = source.fetch(&ticker, 1).await?;
    Ok(posts.len())
}

fn bluesky_guide_text() -> String {
    "\
Bluesky needs a free app password — search requires auth. ~2 minutes:

  1. Create a free account at https://bsky.app if you don't have one.
  2. Sign in, then open:  https://bsky.app/settings/app-passwords
     (Settings → Privacy and Security → App Passwords).
  3. Click \"Add App Password\", name it  openintel , and copy the generated
     password — it is shown only once (format: xxxx-xxxx-xxxx-xxxx).
  4. Put your handle and the app password in your shell (or a gitignored
     .env — see .env.example), then re-run this command:

       export OPENINTEL_BLUESKY_HANDLE=yourname.bsky.social
       export OPENINTEL_BLUESKY_APP_PASSWORD=xxxx-xxxx-xxxx-xxxx
       openintel setup bluesky

openintel reads these only from your environment — it never stores or writes
your credentials to disk."
        .to_string()
}
```

Add imports at the top of setup.rs: `use crate::adapters::sources::bluesky::BlueskySource;`.

- [ ] **Step 4: Update + add tests in setup.rs**

Existing tests: rename `Mode::MissingId` → `Mode::MissingFirst`, `Mode::MissingSecret` → `Mode::MissingSecond`; `partial_text` calls gain the label arg (assert unchanged: contains "`OPENINTEL_REDDIT_CLIENT_ID` is not set"); `verify_ok_text` calls gain `("Reddit", n, "openintel analyze GME --enable-reddit")`; `verify_err_text` calls gain `REDDIT_UNAUTHORIZED_HINT`. New tests:

```rust
    #[test]
    fn bluesky_guide_text_contains_every_load_bearing_instruction() {
        let text = bluesky_guide_text();
        assert!(text.contains("https://bsky.app/settings/app-passwords"));
        assert!(text.contains("Add App Password"));
        assert!(text.contains("export OPENINTEL_BLUESKY_HANDLE="));
        assert!(text.contains("export OPENINTEL_BLUESKY_APP_PASSWORD="));
        assert!(text.contains("never stores"));
    }

    #[test]
    fn partial_text_is_source_aware() {
        let text = partial_text("Bluesky", "OPENINTEL_BLUESKY_APP_PASSWORD");
        assert!(text.contains("Bluesky is half-configured"));
        assert!(text.contains("OPENINTEL_BLUESKY_APP_PASSWORD is not set"));
        assert!(text.contains("openintel setup bluesky"));
    }

    #[test]
    fn verify_err_text_uses_per_source_unauthorized_hint() {
        let unauthorized = DomainError::SourceFailure {
            name: "bluesky".into(),
            message: "unauthorized — check handle/app password".into(),
        };
        let text = verify_err_text(&unauthorized, BLUESKY_UNAUTHORIZED_HINT);
        assert!(text.contains("app-passwords"));
        assert!(!text.contains("prefs/apps"));
    }
```

- [ ] **Step 5: Verify, smoke-test, commit**

Run: `cargo build --all-targets && cargo test && cargo clippy -- -D warnings && cargo fmt`
Expected: green.

Run: `env -u OPENINTEL_BLUESKY_HANDLE -u OPENINTEL_BLUESKY_APP_PASSWORD cargo run -q -- setup bluesky; echo "exit=$?"`
Expected: the bluesky walkthrough, `exit=1`.

Run: `env -u OPENINTEL_BLUESKY_APP_PASSWORD OPENINTEL_BLUESKY_HANDLE=x.bsky.social cargo run -q -- setup bluesky; echo "exit=$?"`
Expected: `⚠  Bluesky is half-configured: OPENINTEL_BLUESKY_APP_PASSWORD is not set.` …, `exit=1`.

```bash
git add src/cli/args.rs src/cli/setup.rs
git commit -m "feat(cli): openintel setup bluesky — guided, env-only credential verify

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 7: Docs — README + .env.example

**Files:**
- Modify: `README.md`, `.env.example`

**Interfaces:** none — docs only, but every command shown must actually parse after Task 4.

- [ ] **Step 1: README edits** (line numbers as of Task 6's HEAD; find by content):

1. Line ~9 (Quickstart lead-in): replace "market data comes from Yahoo Finance, social data stays mocked" with "market data comes from Yahoo Finance; Reddit and Bluesky sentiment go live once configured (see below)".
2. Line ~22 (bold caveat): replace with: `> **Market data is live (Yahoo Finance, keyless). Reddit and Bluesky are live when configured (see below). There is no X source (paid API).** `analyze` fetches over the network — offline or unconfigured sources degrade gracefully with a note.`
3. Line ~31 (usage example): `openintel analyze AAPL --enable-reddit --enable-x` → `openintel analyze AAPL --enable-reddit --enable-bluesky`.
4. Line ~39 (flags table): `--enable-reddit/--enable-x/--enable-bluesky` → `--enable-reddit/--enable-bluesky`.
5. After the "Enable the Reddit source" section, add a sibling section:

```markdown
## Enable the Bluesky source (optional)

Bluesky search requires auth — a free app password (any Bluesky account, no fees). One-time setup:

1. Sign in at <https://bsky.app> → Settings → Privacy and Security → **App Passwords** → add one named `openintel` (the value is shown once).
2. Export handle + app password, verify, then run:

```bash
export OPENINTEL_BLUESKY_HANDLE=yourname.bsky.social
export OPENINTEL_BLUESKY_APP_PASSWORD=xxxx-xxxx-xxxx-xxxx
openintel setup bluesky                    # ✅ live-checks your credentials
openintel analyze AAPL --enable-bluesky
```

Not sure where to start? Run `openintel setup bluesky` with neither variable set for a guided walkthrough.
```

6. Line ~114 (risk/status bullet): "social sources still mocked" → "Reddit and Bluesky sentiment live when configured".
7. Line ~133 (Architecture bullet): "…the `RedditSource` (real via OAuth when configured), and mock X/Bluesky sources." → "…the `RedditSource` and `BlueskySource` (real, credential-gated — no mock sources)."
8. Line ~137 (secrets list): replace `OPENINTEL_X_BEARER` with `OPENINTEL_BLUESKY_HANDLE` in the env-var enumeration.

- [ ] **Step 2: `.env.example`** — replace the final two sections (`# --- Market data ---` block stays; the `# --- Not yet wired… ---` block goes) so the file ends with:

```bash
# --- Bluesky (real social source) ---
# Both are REQUIRED to enable Bluesky. Any free account works: bsky.app →
# Settings → Privacy and Security → App Passwords → add one named "openintel"
# (the handle is public; only the app password is secret). Then run:
# `openintel setup bluesky` to verify.
OPENINTEL_BLUESKY_HANDLE=
OPENINTEL_BLUESKY_APP_PASSWORD=

# --- Market data ---
# Yahoo Finance (the current market source) is KEYLESS — leave this unset.
# Reserved for a future keyed market provider.
OPENINTEL_MARKET_API_KEY=
```

Keep the file's header comment and the Reddit block exactly as they are. The `OPENINTEL_X_BEARER=` line and the "Not yet wired to real adapters (mock today)" header MUST be gone.

- [ ] **Step 3: Verify docs commands parse, commit**

Run: `cargo run -q -- analyze --help | grep -c "enable-x"; echo "---"; grep -rn "enable-x\|X_BEARER\|mock" README.md .env.example | grep -vi "no mock" | grep -vi "no X source"`
Expected: `0` from the first command (flag gone from help); second command surfaces no stale claims (inspect any output).

```bash
git add README.md .env.example
git commit -m "docs: Bluesky setup section; drop X and stale mock claims

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```
