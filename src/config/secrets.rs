use secrecy::SecretString;

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

fn secret_from(value: Option<String>) -> Option<SecretString> {
    value.map(|v| SecretString::new(v.into_boxed_str()))
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
    }

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
}
