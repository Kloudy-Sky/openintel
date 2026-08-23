//! The chatter listening set: who is worth hearing, per platform. Lives at
//! `~/.openintel/listening.json`, hand-editable; created with the macro seed
//! on first use. Not a secret — no keychain involvement.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::application::pulse::DEFAULT_PULSE_ACCOUNTS;
use crate::domain::error::DomainError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListeningSet {
    #[serde(default)]
    pub x_accounts: Vec<String>,
    #[serde(default)]
    pub bluesky_accounts: Vec<String>,
    #[serde(default)]
    pub subreddits: Vec<String>,
}

impl Default for ListeningSet {
    fn default() -> Self {
        Self {
            x_accounts: DEFAULT_PULSE_ACCOUNTS
                .iter()
                .map(|s| s.to_string())
                .collect(),
            bluesky_accounts: Vec::new(),
            subreddits: vec![
                "wallstreetbets".into(),
                "stocks".into(),
                "StockMarket".into(),
            ],
        }
    }
}

impl ListeningSet {
    pub fn handles_for(&self, platform: &str) -> &[String] {
        match platform {
            "x" => &self.x_accounts,
            "bluesky" => &self.bluesky_accounts,
            "reddit" => &self.subreddits,
            _ => &[],
        }
    }

    /// Deterministic identity of the set, stored on every baseline line so
    /// velocity against a changed set is flagged, never silently claimed.
    /// Deliberately not a hash — std hashing isn't stable across runs.
    pub fn fingerprint(&self) -> String {
        let part = |label: &str, list: &[String]| {
            let mut sorted: Vec<String> = list.iter().map(|s| s.to_lowercase()).collect();
            sorted.sort();
            format!("{label}:{}", sorted.join(","))
        };
        [
            part("x", &self.x_accounts),
            part("bsky", &self.bluesky_accounts),
            part("r", &self.subreddits),
        ]
        .join("|")
    }
}

pub fn default_path() -> Option<PathBuf> {
    std::env::home_dir().map(|h| h.join(".openintel").join("listening.json"))
}

fn fail(message: impl Into<String>) -> DomainError {
    DomainError::SourceFailure {
        name: "listening".into(),
        message: message.into(),
    }
}

/// Load the set, seeding the file with defaults on first use. `created` tells
/// the caller to mention the file so the user knows where to curate.
pub fn load_or_seed(path: &Path) -> Result<(ListeningSet, bool), DomainError> {
    match std::fs::read_to_string(path) {
        Ok(content) => {
            let set = serde_json::from_str(&content)
                .map_err(|e| fail(format!("bad listening.json ({e}) — fix or delete it")))?;
            Ok((set, false))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let set = ListeningSet::default();
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| fail(e.to_string()))?;
            }
            let json = serde_json::to_string_pretty(&set).map_err(|e| fail(e.to_string()))?;
            std::fs::write(path, json).map_err(|e| fail(e.to_string()))?;
            Ok((set, true))
        }
        Err(e) => Err(fail(format!("cannot read {}: {e}", path.display()))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_then_load_round_trips() {
        let dir = std::env::temp_dir().join(format!("openintel-listen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("listening.json");

        let (set, created) = load_or_seed(&path).unwrap();
        assert!(created);
        assert_eq!(set.x_accounts.len(), 4);
        assert_eq!(set.subreddits[0], "wallstreetbets");

        let (again, created) = load_or_seed(&path).unwrap();
        assert!(!created);
        assert_eq!(again.fingerprint(), set.fingerprint());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fingerprint_is_order_insensitive_but_content_sensitive() {
        let a = ListeningSet {
            x_accounts: vec!["Alpha".into(), "beta".into()],
            bluesky_accounts: vec![],
            subreddits: vec![],
        };
        let b = ListeningSet {
            x_accounts: vec!["BETA".into(), "alpha".into()],
            bluesky_accounts: vec![],
            subreddits: vec![],
        };
        assert_eq!(a.fingerprint(), b.fingerprint());
        let c = ListeningSet {
            x_accounts: vec!["alpha".into()],
            bluesky_accounts: vec![],
            subreddits: vec![],
        };
        assert_ne!(a.fingerprint(), c.fingerprint());
    }
}
