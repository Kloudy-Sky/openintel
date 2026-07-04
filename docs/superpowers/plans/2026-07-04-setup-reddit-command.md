# `openintel setup reddit` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A `setup reddit` CLI subcommand that guides a user through creating Reddit OAuth credentials (when unset), verifies them live (when set), and names the missing variable (when half-set) — env-only, never storing anything.

**Architecture:** One new CLI leaf module `src/cli/setup.rs` holding all copy as pure, unit-tested render helpers plus a pure `plan()` mode selector; the live verify reuses the existing `RedditSource::new` + `fetch` unchanged. `main.rs` gains one dispatch arm; `args.rs` gains the `Setup` subcommand.

**Tech Stack:** Rust, clap (derive + `ValueEnum`), tokio, `secrecy::SecretString`, existing `RedditSource` adapter.

**Spec:** `docs/superpowers/specs/2026-07-04-setup-reddit-command-design.md` — the user-facing copy in this plan is copied from it verbatim.

## Global Constraints

- **Env-only:** credentials come only from `Credentials::from_env()` (`OPENINTEL_REDDIT_CLIENT_ID` / `OPENINTEL_REDDIT_CLIENT_SECRET`); the command never writes to disk and never stores credentials.
- **Secrets stay sealed:** `SecretString`s are handed straight to `RedditSource::new`; `.expose_secret()` is never called in the new code.
- **No adapter changes:** `RedditSource`, `auth.rs`, and the analysis path are untouched.
- **Output:** human output goes to **stdout** via `println!` in `src/cli/setup.rs` — this is a deliberate spec decision (the command is user-facing guidance, not the MCP stdio path; `mcp/` and `adapters/` still must never print). Exit code carries success/failure: `0` only when the live verify succeeds.
- **Hermetic tests:** the live probe is never run in `cargo test`. All new tests are pure (render helpers, `plan()`, arg parsing).
- **Error hint mapping by substring:** the real adapter messages are `"unauthorized — check client id/secret"` (auth.rs:83) and `"rate limited (HTTP 429)"` (reddit/mod.rs:129) — match on `"unauthorized"` and `"rate limited"`.
- **Every commit green:** `cargo build --all-targets && cargo test && cargo clippy -- -D warnings && cargo fmt -- --check` all pass at each commit.

---

### Task 1: The `setup reddit` command end-to-end

**Files:**
- Create: `src/cli/setup.rs`
- Modify: `src/cli/mod.rs` (add `pub mod setup;`)
- Modify: `src/cli/args.rs` (add `Setup` variant, `SetupArgs`, `SetupSource`; two parse tests; remove now-stale `#[allow(irrefutable_let_patterns)]`)
- Modify: `src/main.rs` (add `Command::Setup` dispatch arm)
- Modify: `README.md:48-55` (verify one-liner)
- Test: unit tests inline in `src/cli/setup.rs` and `src/cli/args.rs`

**Interfaces:**
- Consumes (all existing, unchanged):
  - `Credentials { reddit_client_id: Option<SecretString>, reddit_client_secret: Option<SecretString>, .. }` and `Credentials::from_env()` (`src/config/secrets.rs`)
  - `RedditSource::new(client_id: SecretString, client_secret: SecretString) -> Result<Self, DomainError>` (`src/adapters/sources/reddit/mod.rs`)
  - `SocialDataSource::fetch(&self, ticker: &Ticker, limit: usize) -> Result<Vec<SocialPost>, DomainError>` (trait — must be in scope to call `fetch`)
  - `Ticker::parse(raw: &str) -> Result<Self, DomainError>` (`src/domain/entities/ticker.rs`)
  - `DomainError::SourceFailure { name, message }` with `Display` = `"data source '{name}' failed: {message}"` (`src/domain/error.rs`)
- Produces:
  - `pub async fn run(source: SetupSource, credentials: &Credentials) -> std::process::ExitCode` in `crate::cli::setup`
  - `Command::Setup(SetupArgs)`, `pub struct SetupArgs { pub source: SetupSource }`, `#[derive(ValueEnum)] pub enum SetupSource { Reddit }` in `crate::cli::args`

- [ ] **Step 1: Write the failing arg-parsing tests**

In `src/cli/args.rs`, inside the existing `mod tests`, add:

```rust
    #[test]
    fn parses_setup_reddit() {
        let cli = Cli::try_parse_from(["openintel", "setup", "reddit"]).unwrap();
        let Command::Setup(args) = cli.command else {
            panic!("expected setup command");
        };
        assert_eq!(args.source, SetupSource::Reddit);
    }

    #[test]
    fn rejects_unknown_setup_source() {
        assert!(Cli::try_parse_from(["openintel", "setup", "bogus"]).is_err());
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --lib cli::args`
Expected: COMPILE ERROR — `Setup`/`SetupSource` not found.

- [ ] **Step 3: Add the `Setup` subcommand to `src/cli/args.rs`**

Extend the `Command` enum:

```rust
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Analyze a ticker across social + market sources
    Analyze(AnalyzeArgs),

    /// Run as an MCP server over stdio (for AI agents).
    Mcp,

    /// Guided setup + live check for a data source (env-only; never stores credentials)
    Setup(SetupArgs),
}
```

Below `FormatArg`, add:

```rust
#[derive(clap::Args, Debug)]
pub struct SetupArgs {
    /// Which source to set up
    #[arg(value_enum)]
    pub source: SetupSource,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupSource {
    Reddit,
}
```

On the test module, change

```rust
#[cfg(test)]
#[allow(irrefutable_let_patterns)]
mod tests {
```

to

```rust
#[cfg(test)]
mod tests {
```

(the `let Command::Analyze(..) = … else` patterns are now genuinely refutable, so the allow is stale). Leave the existing tests' `unreachable!()` else-arms untouched.

Note: `cargo test` will still fail to build at this point because `main.rs`'s `match cli.command` is now non-exhaustive. Continue to Step 4 before running anything.

- [ ] **Step 4: Create `src/cli/setup.rs` — pure helpers, tests, and the async runner**

Create the file with this exact content:

```rust
//! `openintel setup <source>` — guided, env-only credential setup + live verify.
//!
//! Never stores or writes credentials (see SECURITY.md): it only reads
//! `Credentials::from_env()` and tells the user what to do next. This is the
//! one CLI-leaf module that prints to stdout directly — it IS the user-facing
//! output, and it never runs under the MCP stdio server.

use std::process::ExitCode;

use secrecy::SecretString;

use crate::adapters::sources::reddit::RedditSource;
use crate::cli::args::SetupSource;
use crate::config::secrets::Credentials;
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::ports::social_data_source::SocialDataSource;

/// Entry point for `openintel setup <source>`. Exit code 0 only when the
/// source is fully configured and a live probe succeeds.
pub async fn run(source: SetupSource, credentials: &Credentials) -> ExitCode {
    match source {
        SetupSource::Reddit => setup_reddit(credentials).await,
    }
}

/// Which of the three setup modes applies, given which env vars are set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Verify,
    MissingId,
    MissingSecret,
    Guide,
}

fn plan(id_set: bool, secret_set: bool) -> Mode {
    match (id_set, secret_set) {
        (true, true) => Mode::Verify,
        (false, true) => Mode::MissingId,
        (true, false) => Mode::MissingSecret,
        (false, false) => Mode::Guide,
    }
}

async fn setup_reddit(credentials: &Credentials) -> ExitCode {
    match plan(
        credentials.reddit_client_id.is_some(),
        credentials.reddit_client_secret.is_some(),
    ) {
        Mode::Guide => {
            println!("{}", guide_text());
            ExitCode::FAILURE
        }
        Mode::MissingId => {
            println!("{}", partial_text("OPENINTEL_REDDIT_CLIENT_ID"));
            ExitCode::FAILURE
        }
        Mode::MissingSecret => {
            println!("{}", partial_text("OPENINTEL_REDDIT_CLIENT_SECRET"));
            ExitCode::FAILURE
        }
        Mode::Verify => {
            println!("Checking your Reddit credentials…");
            let (Some(id), Some(secret)) = (
                credentials.reddit_client_id.clone(),
                credentials.reddit_client_secret.clone(),
            ) else {
                // Unreachable: Mode::Verify is returned only when both are set.
                println!("internal error: credentials unavailable");
                return ExitCode::FAILURE;
            };
            match probe(id, secret).await {
                Ok(count) => {
                    println!("{}", verify_ok_text(count));
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    println!("{}", verify_err_text(&e));
                    ExitCode::FAILURE
                }
            }
        }
    }
}

/// One live round trip through the full Reddit path: OAuth token request plus
/// a search. Returns how many posts the test query yielded.
async fn probe(id: SecretString, secret: SecretString) -> Result<usize, DomainError> {
    let source = RedditSource::new(id, secret)?;
    let ticker = Ticker::parse("AAPL")?;
    let posts = source.fetch(&ticker, 1).await?;
    Ok(posts.len())
}

fn guide_text() -> String {
    "\
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
  5. Put them in your shell (or a gitignored .env — see .env.example), then
     re-run this command:

       export OPENINTEL_REDDIT_CLIENT_ID=paste_your_client_id
       export OPENINTEL_REDDIT_CLIENT_SECRET=paste_your_secret
       openintel setup reddit

openintel reads these only from your environment — it never stores or writes
your credentials to disk."
        .to_string()
}

fn partial_text(missing: &str) -> String {
    format!(
        "⚠  Reddit is half-configured: {missing} is not set.\n   \
         Set it (see `openintel setup reddit` with neither set for the full guide), then re-run."
    )
}

fn verify_ok_text(count: usize) -> String {
    let evidence = if count > 0 {
        format!("pulled {count} recent post(s) for a test query")
    } else {
        "credentials work — the test query just had no recent posts, which is fine".to_string()
    };
    format!(
        "✅ Reddit is configured and working ({evidence}).\n   \
         Real Reddit sentiment is active. Try:  openintel analyze GME --enable-reddit"
    )
}

fn verify_err_text(err: &DomainError) -> String {
    let msg = err.to_string();
    let hint = if msg.contains("unauthorized") {
        "Your client id or secret looks wrong. Re-copy both from\n   \
         https://www.reddit.com/prefs/apps (the id is the short string under the app\n   \
         name; the secret is labelled \"secret\")."
    } else if msg.contains("rate limited") {
        "Reddit is rate-limiting right now — wait a minute and re-run."
    } else {
        "Check your internet connection and try again."
    };
    format!("❌ {msg}\n   {hint}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_selects_mode_for_all_credential_combinations() {
        assert_eq!(plan(true, true), Mode::Verify);
        assert_eq!(plan(false, true), Mode::MissingId);
        assert_eq!(plan(true, false), Mode::MissingSecret);
        assert_eq!(plan(false, false), Mode::Guide);
    }

    #[test]
    fn guide_text_contains_every_load_bearing_instruction() {
        let text = guide_text();
        assert!(text.contains("https://www.reddit.com/prefs/apps"));
        assert!(text.contains("\"script\""));
        assert!(text.contains("export OPENINTEL_REDDIT_CLIENT_ID="));
        assert!(text.contains("export OPENINTEL_REDDIT_CLIENT_SECRET="));
        assert!(text.contains("never stores"));
    }

    #[test]
    fn partial_text_names_the_missing_variable() {
        assert!(partial_text("OPENINTEL_REDDIT_CLIENT_ID")
            .contains("OPENINTEL_REDDIT_CLIENT_ID is not set"));
        assert!(partial_text("OPENINTEL_REDDIT_CLIENT_SECRET")
            .contains("OPENINTEL_REDDIT_CLIENT_SECRET is not set"));
    }

    #[test]
    fn verify_ok_text_distinguishes_empty_from_nonempty_results() {
        let some = verify_ok_text(3);
        assert!(some.contains("pulled 3 recent post(s)"));
        let none = verify_ok_text(0);
        assert!(none.contains("no recent posts"));
        for text in [&some, &none] {
            assert!(text.contains("✅ Reddit is configured and working"));
            assert!(text.contains("openintel analyze GME --enable-reddit"));
        }
    }

    #[test]
    fn verify_err_text_maps_known_failures_to_hints() {
        let unauthorized = DomainError::SourceFailure {
            name: "reddit".into(),
            message: "unauthorized — check client id/secret".into(),
        };
        assert!(verify_err_text(&unauthorized).contains("Re-copy both"));

        let rate_limited = DomainError::SourceFailure {
            name: "reddit".into(),
            message: "rate limited (HTTP 429)".into(),
        };
        assert!(verify_err_text(&rate_limited).contains("wait a minute"));

        let other = DomainError::SourceFailure {
            name: "reddit".into(),
            message: "search request failed: connection refused".into(),
        };
        let text = verify_err_text(&other);
        assert!(text.contains("connection refused")); // raw error preserved
        assert!(text.contains("Check your internet connection"));
    }
}
```

- [ ] **Step 5: Register the module**

In `src/cli/mod.rs`:

```rust
pub mod args;
pub mod run;
pub mod setup;
```

- [ ] **Step 6: Dispatch from `src/main.rs`**

Add a third arm to the `match cli.command` (after the `Command::Mcp` arm):

```rust
        Command::Setup(args) => openintel::cli::setup::run(args.source, &credentials).await,
```

- [ ] **Step 7: Run the full verification suite**

Run: `cargo build --all-targets && cargo test && cargo clippy -- -D warnings && cargo fmt`
Expected: build clean, all tests PASS (including the two new args tests and five new setup tests), clippy clean. Run `cargo fmt` (no `--check`) once to normalize formatting, then `cargo fmt -- --check` to confirm.

- [ ] **Step 8: Smoke-test the guide and partial modes locally (no creds needed)**

Run: `env -u OPENINTEL_REDDIT_CLIENT_ID -u OPENINTEL_REDDIT_CLIENT_SECRET cargo run -q -- setup reddit; echo "exit=$?"`
Expected: the full walkthrough text, `exit=1`.

Run: `env -u OPENINTEL_REDDIT_CLIENT_SECRET OPENINTEL_REDDIT_CLIENT_ID=x cargo run -q -- setup reddit; echo "exit=$?"`
Expected: `⚠  Reddit is half-configured: OPENINTEL_REDDIT_CLIENT_SECRET is not set.` …, `exit=1`.

(Do NOT run the live verify mode — it needs real credentials; that's validated manually by the operator per the spec.)

- [ ] **Step 9: Commit the code**

```bash
git add src/cli/setup.rs src/cli/mod.rs src/cli/args.rs src/main.rs
git commit -m "feat(cli): openintel setup reddit — guided, env-only credential verify

Three modes from the environment alone: full walkthrough when neither
cred is set, names the missing var when half-set, live-verifies via the
existing RedditSource (OAuth + one search) when both are set. Exit 0
only on a working setup. Never stores or writes credentials.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

- [ ] **Step 10: README one-liner**

In `README.md`, section "Enable the Reddit source (optional)", replace:

```markdown
1. Create a **script** app at <https://www.reddit.com/prefs/apps> → note the **client id** (under the app name) and **secret**.
2. Export them before running:

```bash
export OPENINTEL_REDDIT_CLIENT_ID=your_client_id
export OPENINTEL_REDDIT_CLIENT_SECRET=your_secret
openintel analyze AAPL --enable-reddit
```
```

with:

```markdown
1. Create a **script** app at <https://www.reddit.com/prefs/apps> → note the **client id** (under the app name) and **secret**.
2. Export them, verify, then run:

```bash
export OPENINTEL_REDDIT_CLIENT_ID=your_client_id
export OPENINTEL_REDDIT_CLIENT_SECRET=your_secret
openintel setup reddit                     # ✅ live-checks your credentials
openintel analyze AAPL --enable-reddit
```

Not sure where to start? Run `openintel setup reddit` with neither variable set for a guided walkthrough.
```

(Keep the surrounding lines — the intro sentence and the "Without these…" paragraph — unchanged.)

- [ ] **Step 11: Commit the docs**

```bash
git add README.md
git commit -m "docs(readme): point Reddit setup at openintel setup reddit

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```
