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

#[derive(Deserialize)]
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
        .basic_auth(
            client_id.expose_secret(),
            Some(client_secret.expose_secret()),
        )
        .header(reqwest::header::USER_AGENT, user_agent)
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body("grant_type=client_credentials")
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 2, 0, 0, 0).unwrap()
    }

    #[test]
    fn parse_token_ok_and_expiry() {
        let body =
            r#"{"access_token":"abc123","token_type":"bearer","expires_in":3600,"scope":"*"}"#;
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
