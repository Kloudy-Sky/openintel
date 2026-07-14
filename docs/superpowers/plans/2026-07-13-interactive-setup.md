# Interactive Setup (Prompt → Verify → Keychain) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `openintel setup <source>` becomes a one-command flow — guide → hidden-input prompts → live verify → persist to the OS keychain — with env vars always overriding.

**Architecture:** A `CredentialStore` port in `config/store.rs` (real adapter = `keyring` crate; in-memory fake for tests); `Credentials::load(store)` resolves env → keychain per field; the interactive loop in `cli/setup.rs` is generic over injected I/O + probe so it is fully unit-testable. Non-TTY keeps today's exact behavior.

**Tech Stack:** Rust, `keyring = "4"` (default features: macOS/Windows/Linux stores), `rpassword = "7"` (hidden input), `std::io::IsTerminal`, secrecy, tokio.

**Spec:** `docs/superpowers/specs/2026-07-13-interactive-setup-design.md` — copy verbatim from it.

## Global Constraints

- **Env always wins**: `Credentials::load` = env var (non-empty) → keychain → None. A store *malfunction* (not absence) warns to stderr and falls back to env — the keychain must never break `analyze` or the MCP server.
- **Keychain written only by `setup`, only after a successful live verify.**
- **Keychain keys = env-var names**, service `"openintel"`: `OPENINTEL_REDDIT_CLIENT_ID`, `OPENINTEL_REDDIT_CLIENT_SECRET`, `OPENINTEL_BLUESKY_HANDLE`, `OPENINTEL_BLUESKY_APP_PASSWORD`. `OPENINTEL_MARKET_API_KEY` stays env-only.
- **New `.expose_secret()` sites in src/: exactly two** — `KeychainStore::set` (keyring takes `&str`) and the handle SecretString→String unwrap in `Credentials::load` (the handle is public info). Nowhere else; prompt input goes straight into `SecretString`.
- **Never print or store secret values in fallback output** — the save-failure fallback prints `export` lines with placeholder values, not the typed secrets (nothing in scrollback).
- **Non-TTY behavior byte-identical to today** (guide/partial/verify, same copy, same exit codes) except credentials come from `Credentials::load`.
- **Interactive caps:** max 3 verify attempts; empty input re-asks; already-configured asks `Replace it? [y/N]` (default N = verify-only).
- **keyring API** (verified from docs.rs 4.1.4): `keyring::v1::Entry::new(service, user) -> Result<Entry>`, `get_password() -> Result<String>`, `set_password(&str)`, `delete_credential()`, error enum `keyring::v1::Error` with a `NoEntry` variant. If the compiler disagrees with these exact paths (e.g. root re-exports), adapt imports only — behavior as specified.
- **Hermetic tests:** no real keychain access in `cargo test` (one `#[ignore]`d round-trip); no network; no TTY needed (injected I/O).
- **stdout discipline:** `println!` only in `src/cli/setup.rs` + `src/main.rs`; store warnings via `eprintln!`.
- **Every commit green:** `cargo build --all-targets && cargo test && cargo clippy -- -D warnings && cargo fmt -- --check`.

---

### Task 1: The `CredentialStore` port + keychain adapter

**Files:**
- Create: `src/config/store.rs`
- Modify: `src/config/mod.rs`, `Cargo.toml` (+`keyring = "4"`)

**Interfaces:**
- Produces: `pub trait CredentialStore { fn get(&self, key: &str) -> Result<Option<SecretString>, StoreError>; fn set(&self, key: &str, value: &SecretString) -> Result<(), StoreError>; fn delete(&self, key: &str) -> Result<(), StoreError>; }`; `pub struct KeychainStore` (+ `new()`, `Default`); `pub struct StoreError(pub String)` (Display + Error); `#[cfg(test)] pub(crate) struct InMemoryStore` with `new()`, `failing()`, and test-visible contents. Tasks 2–3 consume all of these.

- [ ] **Step 1: Add the dependency**

Run: `cargo add keyring@4`
Expected: resolves 4.x with default features (apple/windows/linux stores bundled).

- [ ] **Step 2: Create `src/config/store.rs`**

```rust
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
```

- [ ] **Step 3: Register the module**

`src/config/mod.rs`:

```rust
pub mod secrets;
pub mod settings;
pub mod store;
```

- [ ] **Step 4: Run the store tests**

Run: `cargo test --lib config::store`
Expected: PASS (2 tests, 1 ignored).

- [ ] **Step 5: Full verification + commit**

Run: `cargo build --all-targets && cargo test && cargo clippy -- -D warnings && cargo fmt`
Expected: green.

```bash
git add src/config/store.rs src/config/mod.rs Cargo.toml Cargo.lock
git commit -m "feat(config): CredentialStore port + OS keychain adapter

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 2: `Credentials::load` + composition roots

**Files:**
- Modify: `src/config/secrets.rs`, `src/main.rs:14`, `src/mcp/server.rs:112`

**Interfaces:**
- Consumes: Task 1's `CredentialStore`, `KeychainStore`, `InMemoryStore`, `StoreError`.
- Produces: `Credentials::load(store: &dyn CredentialStore) -> Credentials` — env-over-keychain per field. Both composition roots now call it; Task 3 relies on `main.rs` having a `store` binding in scope.

- [ ] **Step 1: Add `load` to `src/config/secrets.rs`**

Add imports at top: `use crate::config::store::CredentialStore;` and extend the secrecy import to `use secrecy::{ExposeSecret, SecretString};`. Then inside `impl Credentials`, after `from_env`:

```rust
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
            store_get(store, "OPENINTEL_BLUESKY_HANDLE")
                .map(|s| s.expose_secret().to_string())
        });
        c.bluesky_app_password = c
            .bluesky_app_password
            .or_else(|| store_get(store, "OPENINTEL_BLUESKY_APP_PASSWORD"));
        c
    }
```

And below `plain_from`:

```rust
/// Keychain read that treats malfunction as "unavailable" (with a warning)
/// rather than fatal. Absence is a plain None.
fn store_get(store: &dyn CredentialStore, key: &str) -> Option<SecretString> {
    match store.get(key) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("warning: credential store unavailable for {key}: {e}");
            None
        }
    }
}
```

- [ ] **Step 2: Precedence tests** (append to the existing `mod tests`; note env-var tests must not collide with real env — use the store-only paths without setting env, and env-over-store via a var name that IS set: instead, test precedence through `from_env`-independent construction):

```rust
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
        assert_eq!(
            c.reddit_client_id.unwrap().expose_secret(),
            "store-id"
        );
        assert_eq!(
            c.reddit_client_secret.unwrap().expose_secret(),
            "store-secret"
        );
        assert_eq!(c.bluesky_handle.as_deref(), Some("store.bsky.social"));
        assert_eq!(
            c.bluesky_app_password.unwrap().expose_secret(),
            "store-pw"
        );
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
        assert_eq!(c.reddit_client_id.is_some(), env_only.reddit_client_id.is_some());
        assert_eq!(c.bluesky_handle.is_some(), env_only.bluesky_handle.is_some());
    }
```

(Env-over-store precedence is structurally guaranteed by `.or_else` on the
`from_env` result; the skip-guard pattern above avoids mutating the process
environment, which is unsafe under a threaded test runner.)

- [ ] **Step 3: Switch the composition roots**

`src/main.rs` — replace line 14 (`let credentials = Credentials::from_env();`) and its comment with:

```rust
    // Credentials resolve env-first, then the OS keychain (written by `openintel setup`).
    let store = openintel::config::store::KeychainStore::new();
    let credentials = Credentials::load(&store);
```

`src/mcp/server.rs` — replace line 112 (`let credentials = Credentials::from_env();`) with:

```rust
    let store = crate::config::store::KeychainStore::new();
    let credentials = Credentials::load(&store);
```

(Check the file's existing import style; `Credentials` is already imported there.)

- [ ] **Step 4: Verify + commit**

Run: `cargo build --all-targets && cargo test && cargo clippy -- -D warnings && cargo fmt`
Expected: green.

```bash
git add src/config/secrets.rs src/main.rs src/mcp/server.rs
git commit -m "feat(config): Credentials::load — env-first, keychain fallback, in both roots

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 3: Interactive setup flow + `--forget`

**Files:**
- Modify: `src/cli/setup.rs`, `src/cli/args.rs`, `src/main.rs` (Setup dispatch arm), `Cargo.toml` (+`rpassword = "7"`)

**Interfaces:**
- Consumes: Task 1's `CredentialStore` (+`InMemoryStore` in tests); Task 2's `Credentials::load` (already wired in main); existing `probe_reddit`/`probe_bluesky`, `setup_reddit`/`setup_bluesky`, `verify_ok_text`/`verify_err_text`, hint consts.
- Produces: `pub async fn run(source: SetupSource, credentials: &Credentials, store: &dyn CredentialStore, forget: bool) -> ExitCode`; `SetupArgs { source, forget: bool }`.

- [ ] **Step 1: Add the dependency**

Run: `cargo add rpassword@7`

- [ ] **Step 2: `src/cli/args.rs` — the `--forget` flag + test**

```rust
#[derive(clap::Args, Debug)]
pub struct SetupArgs {
    /// Which source to set up
    #[arg(value_enum)]
    pub source: SetupSource,

    /// Remove this source's saved credentials from the OS keychain
    #[arg(long)]
    pub forget: bool,
}
```

Test (in the existing `mod tests`):

```rust
    #[test]
    fn parses_setup_forget_flag() {
        let cli = Cli::try_parse_from(["openintel", "setup", "reddit", "--forget"]).unwrap();
        let Command::Setup(args) = cli.command else {
            panic!("expected setup command");
        };
        assert!(args.forget);
    }
```

- [ ] **Step 3: Rewrite the top of `src/cli/setup.rs`** — module doc, imports, `SourceSpec`, and the new `run`:

Replace the module doc comment (lines 1–6) with:

```rust
//! `openintel setup <source>` — guided credential setup + live verify.
//!
//! Interactive (TTY): condensed guide -> prompts (secret input hidden) ->
//! live verify -> save to the OS keychain (only after ✅; env vars always
//! override). Non-TTY keeps the classic guide/partial/verify behavior.
//! This is the one CLI-leaf module that prints to stdout directly — it IS
//! the user-facing output, and it never runs under the MCP stdio server.
```

Extend the imports:

```rust
use std::io::{BufRead, IsTerminal, Write};
use std::process::ExitCode;

use secrecy::SecretString;

use crate::adapters::sources::bluesky::BlueskySource;
use crate::adapters::sources::reddit::RedditSource;
use crate::cli::args::SetupSource;
use crate::config::secrets::Credentials;
use crate::config::store::CredentialStore;
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::ports::social_data_source::SocialDataSource;
```

Add the per-source spec (below the hint consts):

```rust
/// Everything the shared interactive loop needs to know about one source.
struct SourceSpec {
    label: &'static str,
    first_key: &'static str,
    second_key: &'static str,
    first_prompt: &'static str,
    second_prompt: &'static str,
    condensed_guide: &'static str,
    unauthorized_hint: &'static str,
    try_cmd: &'static str,
}

const REDDIT_SPEC: SourceSpec = SourceSpec {
    label: "Reddit",
    first_key: "OPENINTEL_REDDIT_CLIENT_ID",
    second_key: "OPENINTEL_REDDIT_CLIENT_SECRET",
    first_prompt: "Client id",
    second_prompt: "Client secret",
    condensed_guide: "\
Reddit needs a free OAuth app — there's no keyless access. ~2 minutes:

  1. Sign in to Reddit, then open:  https://www.reddit.com/prefs/apps
  2. Scroll to the bottom and click \"create another app…\"
     (or \"are you a developer? create an app…\").
  3. Fill in the form:
       • name           openintel        (anything is fine)
       • type           select \"script\"  ← this matters
       • redirect uri   http://localhost:8080   (unused, but required)
     Click \"create app\".
  4. On the app that appears:
       • CLIENT ID  — the short string just under the app name
                      (below \"personal use script\")
       • SECRET     — the value labelled \"secret\"
  5. Enter them below — they'll be verified live and saved to your OS
     keychain (plaintext never touches disk).
",
    unauthorized_hint: REDDIT_UNAUTHORIZED_HINT,
    try_cmd: "openintel analyze GME --enable-reddit",
};

const BLUESKY_SPEC: SourceSpec = SourceSpec {
    label: "Bluesky",
    first_key: "OPENINTEL_BLUESKY_HANDLE",
    second_key: "OPENINTEL_BLUESKY_APP_PASSWORD",
    first_prompt: "Handle (e.g. yourname.bsky.social)",
    second_prompt: "App password",
    condensed_guide: "\
Bluesky needs a free app password — search requires auth. ~2 minutes:

  1. Create a free account at https://bsky.app if you don't have one.
  2. Sign in, then open:  https://bsky.app/settings/app-passwords
     (Settings → Privacy and Security → App Passwords).
  3. Click \"Add App Password\", name it  openintel , and copy the generated
     password — it is shown only once (format: xxxx-xxxx-xxxx-xxxx).
  4. Enter your handle and the app password below — they'll be verified live
     and saved to your OS keychain (plaintext never touches disk).
",
    unauthorized_hint: BLUESKY_UNAUTHORIZED_HINT,
    try_cmd: "openintel analyze GME --enable-bluesky",
};
```

Replace `pub async fn run` with:

```rust
/// Entry point for `openintel setup <source>`. Exit code 0 only when the
/// source is verified working (or `--forget` succeeded).
pub async fn run(
    source: SetupSource,
    credentials: &Credentials,
    store: &dyn CredentialStore,
    forget: bool,
) -> ExitCode {
    let spec = match source {
        SetupSource::Reddit => &REDDIT_SPEC,
        SetupSource::Bluesky => &BLUESKY_SPEC,
    };

    if forget {
        return forget_source(spec, store);
    }

    if !std::io::stdin().is_terminal() {
        // Piped / CI: the classic guide/partial/verify behavior, unchanged.
        return match source {
            SetupSource::Reddit => setup_reddit(credentials).await,
            SetupSource::Bluesky => setup_bluesky(credentials).await,
        };
    }

    let configured = match source {
        SetupSource::Reddit => {
            credentials.reddit_client_id.is_some() && credentials.reddit_client_secret.is_some()
        }
        SetupSource::Bluesky => {
            credentials.bluesky_handle.is_some() && credentials.bluesky_app_password.is_some()
        }
    };
    let already = configured.then(|| provenance(spec.first_key));

    let mut stdin = std::io::stdin().lock();
    let read_secret = |prompt: &str| {
        rpassword::prompt_password(prompt)
            .map(|s| SecretString::new(s.into_boxed_str()))
    };
    let mut io = SetupIo {
        input: &mut stdin,
        read_secret: &read_secret,
    };

    let outcome = match source {
        SetupSource::Reddit => {
            run_interactive(&mut io, store, spec, already, |first, secret| {
                probe_reddit(SecretString::new(first.into_boxed_str()), secret)
            })
            .await
        }
        SetupSource::Bluesky => {
            run_interactive(&mut io, store, spec, already, probe_bluesky).await
        }
    };

    match outcome {
        InteractiveOutcome::Done(Outcome::Success) => ExitCode::SUCCESS,
        InteractiveOutcome::Done(Outcome::Failure) => ExitCode::FAILURE,
        InteractiveOutcome::VerifyExisting => match source {
            SetupSource::Reddit => setup_reddit(credentials).await,
            SetupSource::Bluesky => setup_bluesky(credentials).await,
        },
    }
}

/// Where the already-configured credentials came from, for the replace-ask.
fn provenance(first_key: &str) -> &'static str {
    if std::env::var(first_key)
        .map(|v| !v.is_empty())
        .unwrap_or(false)
    {
        "the environment"
    } else {
        "the OS keychain"
    }
}

fn forget_source(spec: &SourceSpec, store: &dyn CredentialStore) -> ExitCode {
    match forget_outcome(spec, store) {
        Outcome::Success => ExitCode::SUCCESS,
        Outcome::Failure => ExitCode::FAILURE,
    }
}

fn forget_outcome(spec: &SourceSpec, store: &dyn CredentialStore) -> Outcome {
    for key in [spec.first_key, spec.second_key] {
        if let Err(e) = store.delete(key) {
            println!("❌ could not remove {key} from the OS keychain: {e}");
            return Outcome::Failure;
        }
    }
    println!(
        "Removed {} credentials from the OS keychain. (Env vars, if set, still apply.)",
        spec.label
    );
    Outcome::Success
}
```

- [ ] **Step 4: The interactive loop** (add below `forget_source`):

```rust
/// Injected I/O so the interactive loop is unit-testable without a TTY.
struct SetupIo<'a> {
    input: &'a mut dyn BufRead,
    read_secret: &'a dyn Fn(&str) -> std::io::Result<SecretString>,
}

/// `std::process::ExitCode` has no `PartialEq`, so the loop reports this
/// testable outcome and `run()` maps it to an exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Success,
    Failure,
}

enum InteractiveOutcome {
    Done(Outcome),
    /// User declined to replace existing creds -> caller runs the classic verify.
    VerifyExisting,
}

const MAX_ATTEMPTS: usize = 3;

async fn run_interactive<F, Fut>(
    io: &mut SetupIo<'_>,
    store: &dyn CredentialStore,
    spec: &SourceSpec,
    already_configured_from: Option<&'static str>,
    probe: F,
) -> InteractiveOutcome
where
    F: Fn(String, SecretString) -> Fut,
    Fut: std::future::Future<Output = Result<usize, DomainError>>,
{
    if let Some(source_of_truth) = already_configured_from {
        println!(
            "{} is already configured (from {source_of_truth}).",
            spec.label
        );
        match read_visible(io, "Replace it? [y/N]: ") {
            Ok(ans) if matches!(ans.trim().to_lowercase().as_str(), "y" | "yes") => {}
            Ok(_) => return InteractiveOutcome::VerifyExisting,
            Err(_) => return InteractiveOutcome::Done(Outcome::Failure),
        }
    }

    println!("{}", spec.condensed_guide);

    for attempt in 1..=MAX_ATTEMPTS {
        let Ok(first) = prompt_nonempty_visible(io, spec.first_prompt) else {
            return InteractiveOutcome::Done(Outcome::Failure);
        };
        let Ok(secret) = prompt_nonempty_secret(io, spec.second_prompt) else {
            return InteractiveOutcome::Done(Outcome::Failure);
        };

        println!("Checking your {} credentials…", spec.label);
        match probe(first.clone(), secret.clone()).await {
            Ok(count) => {
                let first_secret = SecretString::new(first.into_boxed_str());
                let saved = store
                    .set(spec.first_key, &first_secret)
                    .and_then(|()| store.set(spec.second_key, &secret));
                return InteractiveOutcome::Done(match saved {
                    Ok(()) => {
                        println!("{}", verify_ok_text(spec.label, count, spec.try_cmd));
                        println!(
                            "   Saved to your OS keychain — you're set. (Env vars still override.)"
                        );
                        Outcome::Success
                    }
                    Err(e) => {
                        println!("{}", verify_ok_text(spec.label, count, spec.try_cmd));
                        println!(
                            "⚠  Verified, but saving to the OS keychain failed: {e}\n   \
                             Fall back to environment variables (with your real values):\n\n   \
                             export {}=paste_your_value\n   export {}=paste_your_value",
                            spec.first_key, spec.second_key
                        );
                        Outcome::Failure
                    }
                });
            }
            Err(e) => {
                println!("{}", verify_err_text(&e, spec.unauthorized_hint));
                if attempt < MAX_ATTEMPTS {
                    println!("Let's try again ({} of {MAX_ATTEMPTS}).", attempt + 1);
                }
            }
        }
    }
    println!(
        "Still not verified after {MAX_ATTEMPTS} attempts — double-check the values and re-run `openintel setup`."
    );
    InteractiveOutcome::Done(Outcome::Failure)
}

/// Visible prompt (identifier-like values). Re-asks on empty input.
fn prompt_nonempty_visible(io: &mut SetupIo<'_>, label: &str) -> std::io::Result<String> {
    loop {
        let ans = read_visible(io, &format!("{label}: "))?;
        let trimmed = ans.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
        println!("Please enter a value.");
    }
}

/// Hidden prompt (secret-like values). Re-asks on empty input.
fn prompt_nonempty_secret(io: &mut SetupIo<'_>, label: &str) -> std::io::Result<SecretString> {
    use secrecy::ExposeSecret as _;
    loop {
        let secret = (io.read_secret)(&format!("{label} (input hidden): "))?;
        if !secret.expose_secret().trim().is_empty() {
            return Ok(secret);
        }
        println!("Please enter a value.");
    }
}

fn read_visible(io: &mut SetupIo<'_>, prompt: &str) -> std::io::Result<String> {
    print!("{prompt}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    io.input.read_line(&mut line)?;
    Ok(line)
}
```

**Note on the one `expose_secret` here:** `prompt_nonempty_secret` peeks only
to reject empty input and the value never leaves the function un-wrapped.
This is a third in-src expose site beyond the two the spec names — if you can
check emptiness without it you may, but `SecretString` offers no length API,
so this is accepted; keep it to exactly this emptiness check.

- [ ] **Step 5: Update the Setup dispatch arm in `src/main.rs`**

```rust
        Command::Setup(args) => {
            openintel::cli::setup::run(args.source, &credentials, &store, args.forget).await
        }
```

- [ ] **Step 6: Interactive-loop tests** (append to `src/cli/setup.rs` `mod tests`):

```rust
    use crate::config::store::{CredentialStore, InMemoryStore};
    use secrecy::ExposeSecret;
    use std::io::Cursor;

    fn scripted<'a>(
        input: &'a mut Cursor<&'static str>,
        secrets: &'a dyn Fn(&str) -> std::io::Result<SecretString>,
    ) -> SetupIo<'a> {
        SetupIo {
            input,
            read_secret: secrets,
        }
    }

    fn ok_secret(_prompt: &str) -> std::io::Result<SecretString> {
        Ok(SecretString::new("s3cret".to_string().into_boxed_str()))
    }

    #[tokio::test]
    async fn interactive_happy_path_saves_both_keys() {
        let store = InMemoryStore::new();
        let mut input = Cursor::new("my-id\n");
        let mut io = scripted(&mut input, &ok_secret);
        let outcome = run_interactive(&mut io, &store, &REDDIT_SPEC, None, |_first, _secret| {
            std::future::ready(Ok(2usize))
        })
        .await;
        assert!(matches!(outcome, InteractiveOutcome::Done(Outcome::Success)));
        let map = store.map.borrow();
        assert_eq!(
            map.get("OPENINTEL_REDDIT_CLIENT_ID").unwrap().expose_secret(),
            "my-id"
        );
        assert_eq!(
            map.get("OPENINTEL_REDDIT_CLIENT_SECRET")
                .unwrap()
                .expose_secret(),
            "s3cret"
        );
    }

    #[tokio::test]
    async fn interactive_three_failures_exits_one_and_saves_nothing() {
        let store = InMemoryStore::new();
        let mut input = Cursor::new("id1\nid2\nid3\n");
        let mut io = scripted(&mut input, &ok_secret);
        let outcome = run_interactive(&mut io, &store, &REDDIT_SPEC, None, |_f, _s| {
            std::future::ready(Err(DomainError::SourceFailure {
                name: "reddit".into(),
                message: "unauthorized — check client id/secret".into(),
            }))
        })
        .await;
        assert!(matches!(outcome, InteractiveOutcome::Done(Outcome::Failure)));
        assert!(store.map.borrow().is_empty());
    }

    #[tokio::test]
    async fn interactive_empty_input_reasks_then_succeeds() {
        let store = InMemoryStore::new();
        // First visible answer empty -> re-ask -> then a real id.
        let mut input = Cursor::new("\nreal-id\n");
        let mut io = scripted(&mut input, &ok_secret);
        let outcome = run_interactive(&mut io, &store, &REDDIT_SPEC, None, |first, _s| {
            std::future::ready(if first == "real-id" { Ok(0) } else { Err(DomainError::NoData) })
        })
        .await;
        assert!(matches!(outcome, InteractiveOutcome::Done(Outcome::Success)));
    }

    #[tokio::test]
    async fn interactive_replace_declined_verifies_existing() {
        let store = InMemoryStore::new();
        let mut input = Cursor::new("n\n");
        let mut io = scripted(&mut input, &ok_secret);
        let outcome = run_interactive(
            &mut io,
            &store,
            &REDDIT_SPEC,
            Some("the OS keychain"),
            |_f, _s| std::future::ready(Ok(1)),
        )
        .await;
        assert!(matches!(outcome, InteractiveOutcome::VerifyExisting));
        assert!(store.map.borrow().is_empty());
    }

    #[tokio::test]
    async fn interactive_save_failure_is_exit_one_with_fallback() {
        let store = InMemoryStore::failing();
        let mut input = Cursor::new("my-id\n");
        let mut io = scripted(&mut input, &ok_secret);
        let outcome = run_interactive(&mut io, &store, &REDDIT_SPEC, None, |_f, _s| {
            std::future::ready(Ok(1usize))
        })
        .await;
        assert!(matches!(outcome, InteractiveOutcome::Done(Outcome::Failure)));
    }

    #[test]
    fn forget_is_idempotent_and_removes_keys() {
        let store = InMemoryStore::new()
            .seed("OPENINTEL_REDDIT_CLIENT_ID", "x")
            .seed("OPENINTEL_REDDIT_CLIENT_SECRET", "y");
        assert_eq!(forget_outcome(&REDDIT_SPEC, &store), Outcome::Success);
        assert!(store.map.borrow().is_empty());
        // Second run: nothing left to delete, still success.
        assert_eq!(forget_outcome(&REDDIT_SPEC, &store), Outcome::Success);
    }

    #[test]
    fn condensed_guides_have_no_export_lines() {
        for spec in [&REDDIT_SPEC, &BLUESKY_SPEC] {
            assert!(!spec.condensed_guide.contains("export "));
            assert!(spec.condensed_guide.contains("keychain"));
        }
        assert!(REDDIT_SPEC.condensed_guide.contains("prefs/apps"));
        assert!(BLUESKY_SPEC.condensed_guide.contains("app-passwords"));
    }
```

- [ ] **Step 7: Full verification + smoke tests**

Run: `cargo build --all-targets && cargo test && cargo clippy -- -D warnings && cargo fmt`
Expected: green.

Run: `echo "" | cargo run -q -- setup reddit; echo "exit=$?"`
Expected: non-TTY path → classic guide (or verify if your env has creds), NOT a prompt.

Run: `cargo run -q -- setup reddit --forget; echo "exit=$?"`
Expected: `Removed Reddit credentials from the OS keychain. …`, `exit=0`.

- [ ] **Step 8: Commit**

```bash
git add src/cli/setup.rs src/cli/args.rs src/main.rs Cargo.toml Cargo.lock
git commit -m "feat(cli): interactive setup — prompt, live verify, keychain save, --forget

TTY runs the gh-auth-login flow (hidden secret input via rpassword, 3-attempt
cap, replace-ask for rotation, save only after a successful probe). Non-TTY
keeps the classic guide/partial/verify behavior byte-for-byte.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 4: Docs — README, SECURITY.md, .env.example

**Files:**
- Modify: `README.md`, `SECURITY.md`, `.env.example`

- [ ] **Step 1: README** (find by content, not line number):

1. Quickstart: after the install lines, replace any "Enable real sentiment" pointer with:
   > Enable real sentiment (optional, ~2 min each): run `openintel setup reddit` and `openintel setup bluesky` — each walks you through creating free credentials, verifies them live, and saves them to your OS keychain.
2. "Enable the Reddit source (optional)" section: replace the numbered steps + export block with:

```markdown
Run `openintel setup reddit` — it walks you through creating a free script app, verifies your credentials live, and saves them to your OS keychain. Rotate by re-running it; remove with `openintel setup reddit --forget`.

<details>
<summary>CI / power users: environment variables instead</summary>

Env vars always override the keychain. Create a **script** app at <https://www.reddit.com/prefs/apps>, then:

```bash
export OPENINTEL_REDDIT_CLIENT_ID=your_client_id
export OPENINTEL_REDDIT_CLIENT_SECRET=your_secret
openintel setup reddit   # non-interactive when piped; verifies from env
```

</details>
```

3. "Enable the Bluesky source (optional)" section: same treatment with `openintel setup bluesky`, <https://bsky.app/settings/app-passwords>, and `OPENINTEL_BLUESKY_HANDLE`/`OPENINTEL_BLUESKY_APP_PASSWORD`.
4. The secrets sentence in Architecture: extend to "…come from environment variables (…) or the OS keychain (written only by `openintel setup` after a live verify; env always wins), wrapped in `SecretString` — plaintext never touches disk, never logged."

- [ ] **Step 2: SECURITY.md** — replace the first paragraph of "Credentials & secrets" with:

```markdown
openintel reads credentials from **environment variables** (see
[`.env.example`](.env.example)) or from your **OS keychain** (macOS Keychain /
Windows Credential Manager / Linux secret-service). The keychain is written
only by `openintel setup <source>` — after a successful live verification —
and env vars always take precedence. Plaintext credentials never touch disk:
interactive setup reads secrets with hidden input (nothing lands in shell
history or scrollback) directly into `secrecy::SecretString` (redacted in
debug output, zeroized on drop), and they are never logged. Remove stored
credentials anytime with `openintel setup <source> --forget`. When openintel
runs as an MCP server, credentials stay in its process — the connected AI
agent never sees them.
```

(Keep the "Never commit real credentials" bullets and the rest unchanged.)

- [ ] **Step 3: `.env.example`** — replace the header comment (first 6 lines) with:

```bash
# openintel credentials — env-var template (CI / power users).
#
# Most users don't need this file: run `openintel setup reddit` /
# `openintel setup bluesky` for a guided flow that verifies live and saves
# to your OS keychain. Env vars, when set, always OVERRIDE the keychain.
# If you do use a .env: keep it gitignored, load via direnv or
# `set -a; . ./.env; set +a`, and NEVER commit real values.
```

- [ ] **Step 4: Verify + commit**

Run: `cargo test 2>&1 | tail -3` (docs-only sanity) and `grep -n "export OPENINTEL" README.md | head` (exports now only inside the details blocks).

```bash
git add README.md SECURITY.md .env.example
git commit -m "docs: interactive setup is the recommended flow; env demoted to CI/power users

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```
