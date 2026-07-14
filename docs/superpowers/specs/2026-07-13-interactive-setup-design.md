# Interactive `openintel setup <source>` — Prompt, Verify, Keychain — Design

**Date:** 2026-07-13
**Status:** Draft — awaiting user review

## Goal

`openintel setup reddit` (and `bluesky`) becomes a complete, one-command flow —
gh-auth-login style: condensed guide → prompt for credentials (secret input
hidden) → **live verify** → persist to the **OS keychain** → done. No `export`
dance, no `.env` required, nothing to paste twice. Driven by real first-user
feedback: "can I just run setup and be guided to enter my id and secret?"

## Security posture (SECURITY.md change)

- Secrets live in the **OS keychain** (macOS Keychain / Windows Credential
  Manager / Linux secret-service via the `keyring` crate — encrypted at rest,
  OS-access-gated) **or** in environment variables. Plaintext never touches
  disk.
- **Env always wins** — keychain is a fallback. CI and power users see zero
  behavior change; `.env`+direnv stays documented but demoted to "CI/power
  users".
- Hidden prompt input (rpassword) goes straight into `SecretString` — nothing
  in shell history or scrollback. This is strictly better than the `export`
  lines we currently teach (those land in history).
- The keychain is written **only after a successful live verify** — it never
  holds known-bad credentials — and only by the `setup` command.

## Architecture

### 1. `src/config/store.rs` (new) — the credential-store port

```rust
pub trait CredentialStore {
    /// Ok(None) = not present. Errors are for store malfunction only.
    fn get(&self, key: &str) -> Result<Option<SecretString>, StoreError>;
    fn set(&self, key: &str, value: &SecretString) -> Result<(), StoreError>;
    /// Idempotent: deleting an absent key is Ok.
    fn delete(&self, key: &str) -> Result<(), StoreError>;
}
```

- `StoreError(String)` — a thin wrapper; message only, never the value.
- **`KeychainStore`** — real adapter over `keyring = "4"` (default features
  bundle the macOS/Windows/Linux stores): `keyring::v1::Entry::new("openintel",
  key)` then `get_password` / `set_password` / `delete_credential`. The
  `NoEntry` error variant maps to `Ok(None)`; `delete` maps it to `Ok(())`.
  `get_password`'s `String` is wrapped into `SecretString` immediately.
  `.expose_secret()` is called at exactly one new site: inside
  `KeychainStore::set` (the keyring API takes `&str`).
- **`InMemoryStore`** (`#[cfg(test)]`) — `RefCell<HashMap<String, String>>`
  fake for hermetic tests.
- Keys are the env-var names, 1:1: `OPENINTEL_REDDIT_CLIENT_ID`,
  `OPENINTEL_REDDIT_CLIENT_SECRET`, `OPENINTEL_BLUESKY_HANDLE`,
  `OPENINTEL_BLUESKY_APP_PASSWORD`. (`OPENINTEL_MARKET_API_KEY` stays env-only
  — it is reserved/unused; YAGNI.)

### 2. `Credentials::load` — resolution with precedence

`src/config/secrets.rs` gains:

```rust
pub fn load(store: &dyn CredentialStore) -> Self
```

Per field: env var if set (non-empty) → else keychain value → else `None`.
A store **malfunction** (not absence) prints one `eprintln!` warning naming the
key and resolves that field from env only — the keychain must never be able to
break `analyze` or the MCP server. `from_env()` remains (pure-env; used by
`load` internally and directly by tests).

Both composition roots (`main.rs`, `mcp::server::serve`) construct a
`KeychainStore` and switch from `Credentials::from_env()` to
`Credentials::load(&store)` — so setup-once works for the CLI **and** the MCP
server with zero shell configuration. (Docs note: on macOS the first keychain
read from a new binary may show an OS permission prompt — normal.)

### 3. Interactive flow — `src/cli/setup.rs`

TTY detection: `std::io::stdin().is_terminal()` (`std::io::IsTerminal`, no new
dependency).

**TTY mode** (a human at the keyboard):

1. If the source already resolves (env or keychain), say where from and ask
   `Reddit is already configured (from the OS keychain). Replace it? [y/N]` —
   `N`/enter → run today's verify against the resolved creds and exit with its
   code (re-check without retyping); `y` → continue.
2. Print the **condensed guide** (the existing walkthrough minus its step-5
   export instructions — replaced by "then come back here").
3. Prompt:
   - identifier-like value (client id / handle): visible `read_line` prompt.
   - secret-like value (client secret / app password): **hidden** via
     `rpassword::prompt_password` — read directly into `SecretString`.
   - Empty input → re-ask (same prompt, one nudge line).
4. `Checking your Reddit credentials…` → the existing live probe.
5. **✅** → `store.set` both keys →
   `✅ Verified and saved to your OS keychain — you're set. (Env vars still override.)`
   `   Try:  openintel analyze GME --enable-reddit` → exit 0.
   If the save fails: print the verify success, the store error, and the two
   `export` lines as a fallback → exit 1 (verified but not persisted ≠ set up).
6. **❌** → existing `verify_err_text` hint → re-prompt from step 3, max **3**
   attempts total → exit 1 after the third failure. Ctrl-C anytime; nothing is
   saved before step 5.

**Non-TTY mode** (piped, CI, scripts): exactly today's three-mode behavior
(guide / partial / verify) — no prompts, same copy, same exit codes — except
credentials come from `Credentials::load` (so a keychain-configured machine
verifies in scripts too).

**`--forget`**: `openintel setup reddit --forget` deletes that source's two
keychain keys (idempotent), prints
`Removed Reddit credentials from the OS keychain. (Env vars, if set, still apply.)`,
exit 0. Added to `SetupArgs` as `#[arg(long)] forget: bool`.

### 4. Prompt plumbing (testability)

The prompt loop is written against injected I/O so tests never need a TTY:

```rust
struct SetupIo<'a> {
    input: &'a mut dyn BufRead,             // visible-input reads
    read_secret: &'a dyn Fn(&str) -> std::io::Result<SecretString>, // hidden reads
}
```

Production wires stdin + `rpassword::prompt_password`; tests wire a
`Cursor<&str>` script + a closure returning canned secrets. The live probe is
injected as a closure too (`&dyn Fn(id, secret) -> Result<usize, DomainError>`
— async wrapped at the call site), so the full interactive loop — replace-ask,
re-prompt on empty, 3-attempt cap, save-on-success, save-failure fallback — is
unit-tested with zero network and zero keychain.

## Error handling

- Keychain malfunction during `load`: warn + fall back to env, never fatal.
- Keychain malfunction during setup save: verified-but-unsaved → export-lines
  fallback + exit 1.
- Probe failures: existing `verify_err_text` hints unchanged.
- `--forget` on a machine with no stored creds: success (idempotent).

## Testing (hermetic)

- `KeychainStore` mapping is thin and *not* unit-tested against a real
  keychain (would mutate the developer's OS store); one `#[ignore]`d
  round-trip test (`set → get → delete`) for manual/live runs.
- `InMemoryStore` used everywhere else:
  - `Credentials::load` precedence: env-only, store-only, env-over-store,
    empty-env-falls-to-store, store-error-warns-and-falls-back.
  - Interactive loop (scripted I/O): happy path saves both keys; wrong creds
    3× exits 1 and saves nothing; empty input re-asks; replace-prompt `N` runs
    verify-only; save-failure prints export fallback.
  - `--forget` deletes both keys; idempotent on empty store.
- Arg parsing: `setup reddit --forget`.
- Existing non-TTY render/mode tests unchanged.

## Docs

- README Quickstart: "Enable real sentiment: run `openintel setup reddit` and
  `openintel setup bluesky` — each walks you through it and verifies live."
  The manual `export` blocks in both "Enable the …" sections shrink to the
  CI/power-user alternative.
- SECURITY.md: keychain paragraph per the posture section above.
- `.env.example` header: note it is the CI/power-user path; `openintel setup`
  is the recommended flow.

## Non-goals (YAGNI)

- No `--token`-style flag input (secrets on argv leak via `ps`/history).
- No credential expiry/rotation reminders.
- No keychain storage for `OPENINTEL_MARKET_API_KEY` (reserved, unused).
- No custom keychain service naming/config.
- No Windows/Linux CI testing of the real keychain (adapter is thin; the
  `keyring` crate owns platform correctness).

## Files

**Create**
- `src/config/store.rs`
- this spec

**Modify**
- `src/config/mod.rs` (register `store`)
- `src/config/secrets.rs` (`Credentials::load`)
- `src/cli/setup.rs` (interactive flow, `--forget`, SetupIo)
- `src/cli/args.rs` (`forget` flag)
- `src/main.rs`, `src/mcp/server.rs` (composition roots → `Credentials::load`)
- `Cargo.toml` (`keyring = "4"`, `rpassword = "7"`)
- `README.md`, `SECURITY.md`, `.env.example`
