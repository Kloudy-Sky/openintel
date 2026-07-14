//! Credential-store port: the OS keychain behind a small trait so tests can
//! use an in-memory fake and the keychain can never break the analysis path.
//!
//! Written ONLY by `openintel setup` (after a successful live verify); read
//! as a fallback by `Credentials::load` — env vars always win.

use secrecy::{ExposeSecret, SecretString};

/// Service name under which all openintel keys live in the OS store.
const SERVICE: &str = "openintel";

/// Store malfunction (backend unavailable, access denied, …). Absence of a
/// key is NOT an error — `get` returns `Ok(None)` and `delete` is idempotent.
#[derive(Debug)]
pub struct StoreError(pub String);

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for StoreError {}

pub trait CredentialStore {
    /// Ok(None) = key not present.
    fn get(&self, key: &str) -> Result<Option<SecretString>, StoreError>;
    fn set(&self, key: &str, value: &SecretString) -> Result<(), StoreError>;
    /// Idempotent: deleting an absent key is Ok.
    fn delete(&self, key: &str) -> Result<(), StoreError>;
}

/// Real adapter over the OS keychain (macOS Keychain / Windows Credential
/// Manager / Linux secret-service) via the `keyring` crate.
#[derive(Default)]
pub struct KeychainStore;

impl KeychainStore {
    pub fn new() -> Self {
        KeychainStore
    }

    fn entry(key: &str) -> Result<keyring::v1::Entry, StoreError> {
        keyring::v1::Entry::new(SERVICE, key)
            .map_err(|e| StoreError(format!("keychain entry for {key}: {e}")))
    }
}

impl CredentialStore for KeychainStore {
    fn get(&self, key: &str) -> Result<Option<SecretString>, StoreError> {
        match Self::entry(key)?.get_password() {
            Ok(v) => Ok(Some(SecretString::new(v.into_boxed_str()))),
            Err(keyring::v1::Error::NoEntry) => Ok(None),
            Err(e) => Err(StoreError(format!("keychain read for {key}: {e}"))),
        }
    }

    fn set(&self, key: &str, value: &SecretString) -> Result<(), StoreError> {
        Self::entry(key)?
            .set_password(value.expose_secret())
            .map_err(|e| StoreError(format!("keychain write for {key}: {e}")))
    }

    fn delete(&self, key: &str) -> Result<(), StoreError> {
        match Self::entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::v1::Error::NoEntry) => Ok(()),
            Err(e) => Err(StoreError(format!("keychain delete for {key}: {e}"))),
        }
    }
}

/// In-memory fake for hermetic tests. `failing()` errors on every operation
/// (simulates a broken keychain backend).
#[cfg(test)]
pub(crate) struct InMemoryStore {
    pub map: std::cell::RefCell<std::collections::HashMap<String, SecretString>>,
    fail: bool,
}

#[cfg(test)]
impl InMemoryStore {
    pub fn new() -> Self {
        InMemoryStore {
            map: std::cell::RefCell::new(std::collections::HashMap::new()),
            fail: false,
        }
    }

    pub fn failing() -> Self {
        InMemoryStore {
            map: std::cell::RefCell::new(std::collections::HashMap::new()),
            fail: true,
        }
    }

    pub fn seed(self, key: &str, value: &str) -> Self {
        self.map.borrow_mut().insert(
            key.to_string(),
            SecretString::new(value.to_string().into_boxed_str()),
        );
        self
    }
}

#[cfg(test)]
impl CredentialStore for InMemoryStore {
    fn get(&self, key: &str) -> Result<Option<SecretString>, StoreError> {
        if self.fail {
            return Err(StoreError("simulated store failure".into()));
        }
        Ok(self.map.borrow().get(key).cloned())
    }

    fn set(&self, key: &str, value: &SecretString) -> Result<(), StoreError> {
        if self.fail {
            return Err(StoreError("simulated store failure".into()));
        }
        self.map.borrow_mut().insert(key.to_string(), value.clone());
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), StoreError> {
        if self.fail {
            return Err(StoreError("simulated store failure".into()));
        }
        self.map.borrow_mut().remove(key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret(s: &str) -> SecretString {
        SecretString::new(s.to_string().into_boxed_str())
    }

    #[test]
    fn in_memory_round_trip_and_idempotent_delete() {
        let store = InMemoryStore::new();
        assert!(store.get("K").unwrap().is_none());
        store.set("K", &secret("v")).unwrap();
        assert_eq!(store.get("K").unwrap().unwrap().expose_secret(), "v");
        store.delete("K").unwrap();
        assert!(store.get("K").unwrap().is_none());
        store.delete("K").unwrap(); // absent -> still Ok
    }

    #[test]
    fn failing_store_errors_on_every_op() {
        let store = InMemoryStore::failing();
        assert!(store.get("K").is_err());
        assert!(store.set("K", &secret("v")).is_err());
        assert!(store.delete("K").is_err());
    }

    /// Touches the real OS keychain — run manually: `cargo test --ignored keychain_live`
    #[test]
    #[ignore = "mutates the developer's real OS keychain; run with --ignored"]
    fn keychain_live_round_trip() {
        let store = KeychainStore::new();
        let key = "OPENINTEL_TEST_ROUND_TRIP";
        store.set(key, &secret("test-value")).unwrap();
        assert_eq!(
            store.get(key).unwrap().unwrap().expose_secret(),
            "test-value"
        );
        store.delete(key).unwrap();
        assert!(store.get(key).unwrap().is_none());
    }
}
