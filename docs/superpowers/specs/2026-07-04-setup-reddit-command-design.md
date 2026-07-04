# `openintel setup reddit` — Guided Verify Command — Design

**Date:** 2026-07-04
**Status:** Draft — awaiting user review

## Goal

A CLI command that gets *anyone* — including a non-Rust, non-technical user —
from "I want real Reddit sentiment" to a working setup, and tells them plainly
whether it worked. **Env-only:** it reads credentials from the environment and
**never stores them** (consistent with `SECURITY.md`). It has two jobs: *guide*
(when creds are unset) and *verify* (when they're set).

## Command shape

`openintel setup reddit` — a new `setup` subcommand with a source argument
(`reddit` is the only value today; the enum leaves room for future keyed
sources). Running it does the right thing based on what's in the environment —
there are no flags to learn.

## Behavior — three modes

Reads `OPENINTEL_REDDIT_CLIENT_ID` / `OPENINTEL_REDDIT_CLIENT_SECRET` via
`Credentials::from_env()`.

### 1. Both set → **verify** (live)
Builds `RedditSource::new(id, secret)` and runs one probe:
`fetch(&Ticker::parse("AAPL")?, 1)` (this exercises the full path — OAuth token
request **and** a search). No new adapter code.

- **`Ok(posts)`** → success. Exit `0`.
  ```
  Checking your Reddit credentials…
  ✅ Reddit is configured and working (pulled a live test result).
     Real Reddit sentiment is active. Try:  openintel analyze GME --enable-reddit
  ```
  (If `posts` is non-empty, say "pulled N recent post(s) for a test query";
  if empty, say "credentials work — the test query just had no recent posts,
  which is fine.")
- **`Err(e)`** → failure. Exit `1`. Print a plain-English reason + fix hint:
  - message contains `"unauthorized"` → "Your client id or secret looks wrong.
    Re-copy both from https://www.reddit.com/prefs/apps (the id is the short
    string under the app name; the secret is labelled *secret*)."
  - message contains `"rate limited"` → "Reddit is rate-limiting right now —
    wait a minute and re-run."
  - otherwise → print the error and "Check your internet connection and try
    again."

### 2. Exactly one set → **partial** (misconfig)
Exit `1`. Name the missing one:
```
⚠  Reddit is half-configured: OPENINTEL_REDDIT_CLIENT_SECRET is not set.
   Set it (see `openintel setup reddit` with neither set for the full guide), then re-run.
```
(Symmetric for a missing client id.)

### 3. Neither set → **guide** (first run)
Exit `1` (signals "not ready yet"), print the full walkthrough to stdout:

```
Reddit needs a free OAuth app — there's no keyless access. ~2 minutes:

  1. Sign in to Reddit, then open:  https://www.reddit.com/prefs/apps
  2. Scroll to the bottom and click "create another app…"
     (or "are you a developer? create an app…").
  3. Fill in the form:
       • name           openintel        (anything is fine)
       • type           select "script"  ← this matters
       • redirect uri   http://localhost:8080   (unused, but required)
     Click "create app".
  4. On the app that appears:
       • CLIENT ID  — the short string just under the app name
                      (below "personal use script")
       • SECRET     — the value labelled "secret"
  5. Put them in your shell (or a gitignored .env — see .env.example), then
     re-run this command:

       export OPENINTEL_REDDIT_CLIENT_ID=paste_your_client_id
       export OPENINTEL_REDDIT_CLIENT_SECRET=paste_your_secret
       openintel setup reddit

openintel reads these only from your environment — it never stores or writes
your credentials to disk.
```

## Architecture

- `src/cli/args.rs` — add `Setup(SetupArgs)` to `Command`; `SetupArgs { source:
  SetupSource }`; `#[derive(ValueEnum)] enum SetupSource { Reddit }`.
- `src/cli/setup.rs` (new) — the command's logic and copy:
  - `pub async fn run(source: SetupSource, credentials: &Credentials) -> ExitCode`
    dispatches on `source` (only `Reddit` today).
  - `async fn verify_reddit(credentials) -> ExitCode`: matches
    `(reddit_client_id, reddit_client_secret)` → verify / partial / guide.
  - **Pure render helpers** (unit-tested), each returns a `String`:
    `guide_text()`, `partial_text(missing: &str)`, `verify_ok_text(count: usize)`,
    `verify_err_text(&DomainError)`.
  - The live probe: build `RedditSource`, `fetch(&Ticker::parse("AAPL")?, 1)`.
- `src/main.rs` — add a `Command::Setup(args) => cli::setup::run(args.source,
  &credentials).await` arm (main already builds `Credentials::from_env()`).
- `src/cli/mod.rs` — `pub mod setup;`.

The command writes human output to **stdout** (it's the CLI, not the MCP path,
so stdout is fine — this is not part of the MCP stdio server); errors/hints go
to stdout too since the whole command is user-facing guidance (exit code carries
success/failure for scripts).

## Reuse & boundaries

No changes to `RedditSource`, `auth.rs`, or the analysis path. The command is a
thin new CLI leaf that reuses `RedditSource::new` + `fetch` and
`Credentials::from_env`. Secrets stay in `SecretString`; `.expose_secret()` is
only ever called inside `RedditSource` (unchanged) — the setup command hands the
`SecretString`s straight to `RedditSource::new` and never inspects them.

## Error handling

The probe's `DomainError::SourceFailure` is mapped to a friendly hint by
substring (`"unauthorized"`, `"rate limited"`), else the raw message + a generic
hint. `Ticker::parse("AAPL")` is a hardcoded valid symbol, so it cannot fail at
runtime; treat a parse error defensively as an internal error message.

## Testing

Hermetic by default — the live probe is never run in `cargo test`.

- **Pure render helpers** — assert each contains its load-bearing content:
  `guide_text()` contains the `prefs/apps` URL, `script`, both `export` lines,
  and "never stores"; `partial_text("…SECRET")` names the missing var;
  `verify_ok_text(0)` vs `verify_ok_text(3)` differ correctly; `verify_err_text`
  for an `unauthorized` `SourceFailure` contains the re-copy-creds hint, and a
  generic one falls back to the connection hint.
- **Branch selection** — a small pure fn `plan(id: bool, secret: bool) ->
  Mode { Verify, MissingId, MissingSecret, Guide }` is unit-tested for all four
  input combinations (decouples "which mode" from the live probe).
- **Args parsing** — `openintel setup reddit` parses to
  `Command::Setup { source: Reddit }`; `openintel setup bogus` errors.
- **Live probe** — reuses the already-`#[ignore]`d Reddit path; the setup
  command's own live behavior is validated manually with real creds (documented
  in the README), not in CI.

## Docs

README "Enable the Reddit source" section gains a one-liner: after setting the
env vars, run `openintel setup reddit` to verify (and run it with nothing set to
get the guided walkthrough).

## Non-goals (YAGNI)

- **No writing to disk** — no `--write-env`, no credential storage. Env-only.
- **No other sources** — Reddit is the only one needing creds; `SetupSource`
  is an enum so a future keyed source is a one-variant addition.
- **No `--check`/quiet flag** — the exit code already makes it scriptable.
- No interactive prompt that reads the secret via stdin (that risks it landing
  in shell history / scrollback; we guide the user to `export` instead).

## Files

**Create**
- `src/cli/setup.rs`
- `docs/superpowers/specs/2026-07-04-setup-reddit-command-design.md` (this file)

**Modify**
- `src/cli/mod.rs` (`pub mod setup;`)
- `src/cli/args.rs` (`Setup` command + `SetupSource`)
- `src/main.rs` (dispatch arm)
- `README.md` (verify one-liner)
