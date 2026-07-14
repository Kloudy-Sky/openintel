use secrecy::{ExposeSecret, SecretString};

use crate::config::store::CredentialStore;

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

    /// Resolve credentials with precedence: environment variable (non-empty)
    /// -> OS keychain -> unset. A store malfunction warns and falls back to
    /// env-only for that field — the keychain can never break analysis.
    pub fn load(store: &dyn CredentialStore) -> Self {
        let mut c = Credentials::from_env();
        c.reddit_client_id = c
            .reddit_client_id
            .or_else(|| store_get(store, "OPENINTEL_REDDIT_CLIENT_ID"));
        c.reddit_client_secret = c
            .reddit_client_secret
            .or_else(|| store_get(store, "OPENINTEL_REDDIT_CLIENT_SECRET"));
        // The handle is public info (kept as a plain String on Credentials);
        // unwrap the store's SecretString wrapper at this one site.
        c.bluesky_handle = c.bluesky_handle.or_else(|| {
            store_get(store, "OPENINTEL_BLUESKY_HANDLE").map(|s| s.expose_secret().to_string())
        });
        c.bluesky_app_password = c
            .bluesky_app_password
            .or_else(|| store_get(store, "OPENINTEL_BLUESKY_APP_PASSWORD"));
        c
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

/// Keychain read that treats malfunction as "unavailable" (with a warning)
/// rather than fatal. Absence of a key is NOT an error — `get` returns `Ok(None)` and `delete` is idempotent.
fn store_get(store: &dyn CredentialStore, key: &str) -> Option<SecretString> {
    match store.get(key) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("warning: credential store unavailable for {key}: {e}");
            None
        }
    }
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

    #[test]
    fn load_falls_back_to_store_when_env_unset() {
        use crate::config::store::{CredentialStore, InMemoryStore};
        let store = InMemoryStore::new()
            .seed("OPENINTEL_REDDIT_CLIENT_ID", "store-id")
            .seed("OPENINTEL_REDDIT_CLIENT_SECRET", "store-secret")
            .seed("OPENINTEL_BLUESKY_HANDLE", "store.bsky.social")
            .seed("OPENINTEL_BLUESKY_APP_PASSWORD", "store-pw");
        // Guard: these env vars must not leak into the test environment.
        for key in [
            "OPENINTEL_REDDIT_CLIENT_ID",
            "OPENINTEL_REDDIT_CLIENT_SECRET",
            "OPENINTEL_BLUESKY_HANDLE",
            "OPENINTEL_BLUESKY_APP_PASSWORD",
        ] {
            if std::env::var(key).map(|v| !v.is_empty()).unwrap_or(false) {
                eprintln!("skipping load_falls_back_to_store_when_env_unset: {key} set in env");
                return;
            }
        }
        let c = Credentials::load(&store);
        assert_eq!(c.reddit_client_id.unwrap().expose_secret(), "store-id");
        assert_eq!(
            c.reddit_client_secret.unwrap().expose_secret(),
            "store-secret"
        );
        assert_eq!(c.bluesky_handle.as_deref(), Some("store.bsky.social"));
        assert_eq!(c.bluesky_app_password.unwrap().expose_secret(), "store-pw");
        // Unrelated read never touched the store's error path
        let _ = &store as &dyn CredentialStore;
    }

    #[test]
    fn load_survives_a_broken_store() {
        use crate::config::store::InMemoryStore;
        let store = InMemoryStore::failing();
        let c = Credentials::load(&store); // must not panic; falls back to env-only
                                           // Whatever env says is what we get; a broken keychain adds nothing.
        let env_only = Credentials::from_env();
        assert_eq!(
            c.reddit_client_id.is_some(),
            env_only.reddit_client_id.is_some()
        );
        assert_eq!(
            c.bluesky_handle.is_some(),
            env_only.bluesky_handle.is_some()
        );
    }
}
