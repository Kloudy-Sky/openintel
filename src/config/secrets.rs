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
            bluesky_app_password: secret_from(std::env::var("OPENINTEL_BLUESKY_APP_PASSWORD").ok()),
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

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    #[test]
    fn wraps_present_value_and_skips_absent() {
        let some = secret_from(Some("super-token".to_string())).unwrap();
        assert_eq!(some.expose_secret(), "super-token");
        assert!(secret_from(None).is_none());
        assert!(secret_from(Some(String::new())).is_none());
        assert_eq!(
            plain_from(Some("me.bsky.social".into())).as_deref(),
            Some("me.bsky.social")
        );
        assert!(plain_from(Some(String::new())).is_none());
        assert!(plain_from(None).is_none());
    }

    #[test]
    fn debug_does_not_leak_secret() {
        let creds = Credentials {
            reddit_client_id: secret_from(Some("leak-me".to_string())),
            reddit_client_secret: None,
            bluesky_handle: Some("public.bsky.social".into()),
            bluesky_app_password: secret_from(Some("leak-me-too".to_string())),
            market_api_key: None,
        };
        assert!(!format!("{creds:?}").contains("leak-me"));
        assert!(!format!("{creds:?}").contains("leak-me-too"));
    }
}
