# Real Bluesky Social Source (App-Password Auth) + All-Real Social Wiring — Design

**Date:** 2026-07-10
**Status:** Draft — awaiting user review

## Goal

Replace the last mock social data with a real Bluesky source, and make the
social side of openintel honest end-to-end: every wired source is real, or it
isn't wired. Three deliverables in one coherent change:

1. A real **Bluesky adapter** authenticated with a free app password
   (`com.atproto.server.createSession` → `app.bsky.feed.searchPosts`).
2. **No mocks in production wiring** — a no-creds run degrades to a
   market-only report with a "no social sources configured" note.
3. **X excised** — flag, `SourceKind::X`, `x_bearer` credential, and the mock
   modules are deleted. openintel supports exactly what's real: Reddit +
   Bluesky. (X's API is paid-only; re-adding it later is a clean, additive PR.)

Plus `openintel setup bluesky` — the same guided verify the Reddit source got.

## Why app-password auth (verified 2026-07-09)

Keyless search is not viable: `public.api.bsky.app/xrpc/app.bsky.feed.searchPosts`
returns 403 ("administrative rules" via CDN) while other keyless endpoints
(e.g. `getProfile`) still work; GitHub issues (bsky-docs#332, atproto#3583)
show this flaky-to-blocked since 2024 with no fix. The lexicon itself says
searchPosts "may require authentication … for some service providers."
App passwords are free (any Bluesky account, Settings → App Passwords), have
no fees or approval process, and make the authed path reliable.

## Bluesky adapter — `src/adapters/sources/bluesky/`

Mirrors the Reddit adapter's structure exactly (auth / response / mod).

### `auth.rs` — session management

- `POST https://bsky.social/xrpc/com.atproto.server.createSession` with JSON
  body `{"identifier": <handle>, "password": <app password>}` (field names per
  the atproto lexicon).
- Response: `{ "accessJwt": …, "refreshJwt": …, "handle": …, "did": … }`.
  Only `accessJwt` is used (v1 re-creates the session rather than using
  `refreshJwt` — sessions are needed at most once per ~expiry window, far
  under createSession rate limits).
- **Expiry:** createSession returns no `expires_in`. Decode the JWT payload
  (2nd dot-separated segment, base64url no-pad, via the `base64` crate) and
  read the `exp` claim (unix seconds) — a pure `parse_jwt_exp(&str) ->
  Option<DateTime<Utc>>` function, unit-tested with hand-built tokens. If the
  claim can't be decoded, fall back to a conservative fixed TTL of 10 minutes
  (functional, never wrong-side stale). Refresh 60 s early.
- `CachedToken { bearer: SecretString, expires_at: DateTime<Utc> }` with pure
  `is_expired(now)`, cached in `RwLock<Option<CachedToken>>` with the same
  double-checked-lock single-flight pattern as Reddit's `ensure_token`.
- Error mapping (all `DomainError::SourceFailure { name: "bluesky", … }`):
  - 401 → `"unauthorized — check handle/app password"`
  - 429 → `"rate limited (HTTP 429)"`
  - other non-2xx → `"session request HTTP <status>"`
  (Same substrings the setup command's hints key on — consistent with Reddit.)

### `response.rs` — pure parser

`parse_posts(body: &str, limit: usize, fetched_at: DateTime<Utc>) ->
Result<Vec<SocialPost>, DomainError>` over the searchPosts response
(`{ "posts": [postView, …] }`). Per post (serde structs, all fields
`#[serde(default)]`-tolerant like Reddit's):

- `id` ← `uri`
- `author` ← `author.handle`
- `text` ← `record.text` — through `PostText::parse` (trim, reject empty);
  posts failing it are **skipped**, not fatal. (No truncation logic needed:
  Bluesky posts are ≤300 graphemes, far under the domain's `MAX_POST_LEN`.)
- `created_at` ← `record.createdAt` (RFC 3339); fall back to `indexedAt`;
  fall back to `fetched_at`
- `engagement` ← `likeCount + repostCount + replyCount` (each defaulting 0,
  saturating, clamped to `u32`)
- `source` ← `SourceKind::Bluesky`
- Stop at `limit`; `limit == 0` returns empty without parsing posts.
- Malformed JSON → `SourceFailure("malformed response: …")`.

### `mod.rs` — the source

- `BlueskySource::new(handle: String, app_password: SecretString) ->
  Result<Self, DomainError>` — reqwest client, 10 s timeout, same
  `rust:openintel:v{CARGO_PKG_VERSION}` UA pattern as Reddit. The handle is
  public info (not a secret); the app password stays a `SecretString`,
  `.expose_secret()` called only at the createSession body build (the one
  new call site).
- `SocialDataSource::fetch`: ensure token →
  `GET https://bsky.social/xrpc/app.bsky.feed.searchPosts` with
  `Authorization: Bearer <accessJwt>` and query params (via
  `Url::query_pairs_mut()`, same reqwest-0.13 workaround as Reddit):
  - `q` = the ticker symbol as-is (e.g. `AAPL`) — plain-text query for
    recall; `$`-cashtag syntax is unreliable in Bluesky search
  - `sort` = `latest`
  - `limit` = `min(limit, 100)` (lexicon max)
- Single request, no `cursor` pagination in v1 (matches Reddit; default
  analyze limit is 50).
- Search-call errors: 400/401 → `"unauthorized — check handle/app password"`
  (expired/invalid token surfaces here); 429 → `"rate limited (HTTP 429)"`;
  other non-2xx → `"search request HTTP <status>"`.

## Credentials — `src/config/secrets.rs`

- Add `bluesky_handle: Option<String>` from `OPENINTEL_BLUESKY_HANDLE`
  (plain `String` — a handle is public, and keeping it out of `SecretString`
  means `Debug` redaction still tells the truth). Empty-string env vars are
  already treated as unset for secrets; apply the same `filter` to the handle.
- `bluesky_app_password` already exists (`OPENINTEL_BLUESKY_APP_PASSWORD`).
- **Remove `x_bearer`** (and `OPENINTEL_X_BEARER` from `.env.example`/docs).

## Wiring — `build_social_sources`

```text
reddit   ← both OPENINTEL_REDDIT_* set        (unchanged)
bluesky  ← OPENINTEL_BLUESKY_HANDLE + _APP_PASSWORD set
(nothing else — mock X and mock Bluesky are gone)
```

Partial Bluesky creds (one of the two set) → `eprintln!` warning naming both
vars, source omitted — the same pattern Reddit has. Zero sources configured is
legal: `analyze` already degrades gracefully (market-only report, per-source
"not configured" notes when flags request an unconfigured source).

## X excision (mechanical sweep)

- `src/cli/args.rs`: remove `--enable-x` / `enable_x`; `to_app_config` loses
  the param. No flags → all (now two) sources enabled, unchanged semantics.
- `src/config/settings.rs`: `AppConfig::new` drops `enable_x`.
- `src/domain/values/source_kind.rs`: remove `X` variant + its serde/tests.
- `src/mcp/tools.rs`: `list_sources` / source parsing lose `"x"`.
- Delete `src/adapters/sources/mock_x.rs` and `mock_bluesky.rs`. Test sites
  that used them (`src/application/analyze.rs`, `src/cli/run.rs`,
  `src/mcp/tools.rs` unit tests; `tests/analyze_flow.rs`) define **local test
  doubles** (a small `struct TestSource { kind, posts }` implementing
  `SocialDataSource` — `#[cfg(test)]` in lib modules, plain local type in the
  integration test, which can implement the pub trait itself). No fake
  sources remain in the library's public API.

## `openintel setup bluesky`

- `SetupSource` gains `Bluesky` (the enum was designed for this).
- Same three modes as Reddit, driven by `bluesky_handle` / `bluesky_app_password`:
  - **Guide** (neither set): create a free account at bsky.app if needed →
    Settings → Privacy and Security → App Passwords → "Add App Password" →
    name it `openintel` → copy the generated password (shown once) → export
    `OPENINTEL_BLUESKY_HANDLE` (e.g. `yourname.bsky.social`) and
    `OPENINTEL_BLUESKY_APP_PASSWORD` → re-run. Ends with the same "env-only,
    never stores" line. Exit 1.
  - **Partial** (one set): names the missing variable. Exit 1.
  - **Verify** (both set): `BlueskySource::new` + `fetch(AAPL, 1)` — exercises
    createSession **and** search. ✅ exit 0 / ❌ exit 1 with hints keyed on
    the same `"unauthorized"` / `"rate limited"` substrings.
- `setup.rs` refactors its Reddit-specific helpers just enough to share the
  mode-selection `plan()` and the ✅/⚠/❌ framing; guide/hint copy stays
  per-source (no premature abstraction beyond that).

## Error handling

All network/HTTP/parse failures map to `DomainError::SourceFailure` with
`name: "bluesky"` — the analysis path already treats per-source failures as
non-fatal notes. No panics on network data; no `unwrap` outside tests.

## Testing (hermetic — `cargo test` never touches the network)

- `parse_jwt_exp`: valid token (hand-built base64url payload), missing exp,
  non-JWT garbage → `None`.
- `is_expired`: fresh / expired / within-60s-skew.
- `parse_posts`: happy path (2 posts), skips empty text, `limit` truncation,
  `limit == 0`, missing optional fields (no author/createdAt → fallbacks),
  malformed JSON error, engagement summing/clamp.
- Wiring: `build_social_sources` gating for bluesky (both/partial/none) —
  extends the existing credentials-gating tests; assert mocks are gone (no
  sources when nothing configured).
- Setup: `plan()` reuse, Bluesky guide/partial/verify render helpers (URL,
  both export lines, "never stores"), hint mapping.
- Args: `setup bluesky` parses; `--enable-x` now errors (clap unknown flag —
  covered implicitly, one negative parse test added).
- One `#[ignore]`d live test (`bluesky_live_search`) mirroring Reddit's.
- Setup verify validated manually with real creds.

## Docs

- README: Quickstart caveat becomes "Reddit and Bluesky are live when
  configured; there is no X source (paid API)". New "Enable the Bluesky
  source" section mirroring Reddit's (app-password steps + `openintel setup
  bluesky`). Env-var list updated (add `OPENINTEL_BLUESKY_HANDLE`, drop
  `OPENINTEL_X_BEARER`). Architecture bullet updated.
- `.env.example`: add `OPENINTEL_BLUESKY_HANDLE`, drop `OPENINTEL_X_BEARER`.

## Non-goals (YAGNI)

- No `refreshJwt`/refreshSession flow — re-createSession on expiry is well
  under rate limits at our call cadence.
- No cursor pagination, no `since`/`until`/`lang` filters.
- No keyless/public-endpoint fallback path (verified unreliable).
- No generic "setup any source" abstraction beyond the existing enum.
- No demo mode replacing the mocks.

## Files

**Create**
- `src/adapters/sources/bluesky/{mod.rs,auth.rs,response.rs}`
- this spec

**Modify**
- `src/config/secrets.rs` (+handle, −x_bearer)
- `src/adapters/sources/mod.rs` (wire bluesky, drop mocks)
- `src/cli/args.rs`, `src/config/settings.rs` (−enable_x; +SetupSource::Bluesky)
- `src/cli/setup.rs` (bluesky modes)
- `src/domain/values/source_kind.rs` (−X)
- `src/mcp/tools.rs`, `src/cli/run.rs`, `src/application/analyze.rs` (test doubles, −"x")
- `tests/analyze_flow.rs` (local test double)
- `README.md`, `.env.example`
- `Cargo.toml` (+`base64`)

**Delete**
- `src/adapters/sources/mock_x.rs`, `src/adapters/sources/mock_bluesky.rs`
