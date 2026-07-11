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
    let expires_at =
        parse_jwt_exp(&access_jwt).unwrap_or_else(|| now + Duration::seconds(FALLBACK_TTL_SECS));
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
        format!(
            "{}.{}.{}",
            enc(r#"{"alg":"none"}"#),
            enc(payload),
            enc("sig")
        )
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
