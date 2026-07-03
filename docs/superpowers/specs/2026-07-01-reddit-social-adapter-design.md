# Reddit OAuth Social Adapter — Design

**Date:** 2026-07-01
**Status:** Draft — awaiting user review (open items flagged inline)

## Goal

Add a real Reddit `SocialDataSource` that searches finance subreddits for a
ticker via Reddit's app-only OAuth API and returns real `SocialPost`s,
replacing the mock Reddit fixtures in the production path. Mirrors the Yahoo
market adapter: fetch/parse split, injected via DI, hermetic tests. X and
Bluesky stay mock (X is paid-only; Bluesky is keyless but sparse on tickers —
each is a separate future cycle).

## Why Reddit via OAuth (not keyless)

Verified empirically 2026-07-01: **every** unauthenticated Reddit endpoint
(`www`/`old`/subreddit listing/`oauth` without token, browser UA included)
returns HTTP 403 with Reddit's block page. This is policy (post-2023 API
lockdown), not an IP fluke — there is no keyless path like Yahoo had. Reddit
(r/wallstreetbets, r/stocks, r/options) is the richest retail-trading
sentiment source, so the signal quality justifies a one-time app registration.

## Auth — app-only OAuth (`client_credentials`)

Verified against Reddit's OAuth2 + API wiki (2026-07-01):

- One-time user setup: register a **script** (or web) app at
  `reddit.com/prefs/apps` → `client_id` + `client_secret`.
- Token request: `POST https://www.reddit.com/api/v1/access_token`, body
  `grant_type=client_credentials`, HTTP Basic auth (`client_id`:`client_secret`),
  plus the required `User-Agent` header.
- Response: `{ access_token, token_type: "bearer", expires_in: <seconds>, scope }`.
  No refresh token for app-only.
- API calls: `https://oauth.reddit.com/...` with `Authorization: bearer <token>`
  and the `User-Agent` header.
- **User-Agent is mandatory and enforced.** Format
  `<platform>:<app-id>:<version> (by /u/<user>)`; generic UAs (`python`, `Java`)
  are "drastically limited." The adapter sends a hardcoded descriptive UA:
  `rust:openintel:v{CARGO_PKG_VERSION} (by /u/openintel)`.
  *(Open item C — see below: hardcode vs. env override.)*
- Rate limit: 60 requests/min for OAuth; `X-Ratelimit-{Used,Remaining,Reset}`
  headers report status. One search per ticker is well within budget → no
  rate-limit accounting in v1; a 429 degrades gracefully (below).

## Secret handling

**Model: env-var + `secrecy::SecretString` (consistent with existing
credentials).** *(Open item — user asked about this; keychain is a documented
future opt-in, see below.)*

- New secrets: `OPENINTEL_REDDIT_CLIENT_ID` + `OPENINTEL_REDDIT_CLIENT_SECRET`,
  replacing the unused `OPENINTEL_REDDIT_TOKEN` scaffold in `Credentials`.
- Both wrapped in `secrecy::SecretString` — `Debug`/`Display` redacted (existing
  `debug_does_not_leak_secret` test extended), memory zeroized on drop.
- Env-only: never written to disk, never logged. Sent only to Reddit's token
  endpoint over rustls TLS, as HTTP Basic auth. The derived bearer token lives
  in memory (the cache) and is likewise never logged.
- The MCP AI agent never sees the secrets — they stay in the openintel process
  environment; the agent only calls read-only tools.
- **Threat model (honest):** env vars are readable by same-user processes and
  can leak into shell history if exported inline. Standard for a local
  single-user CLI. Mitigation guidance in the README (gitignored `.env` +
  direnv; `read -s`).
- **Future opt-in (deferred, YAGNI):** OS keychain (macOS Keychain / Linux
  Secret Service / Windows Credential Manager via the `keyring` crate) with a
  keychain→env lookup order. This is localized to `config/secrets.rs` and
  touches no callers, so choosing env-var now does not preclude it.

## Graceful absence

If the Reddit creds are unset, the composition root does not wire the Reddit
source. Enabling `--enable-reddit` (or the MCP `enable_reddit`) without creds
yields the generic per-source note `"reddit enabled but not configured"` (the
env-var setup detail lives in the README); the market snapshot and other social
sources still run. (This reuses the application layer's existing per-source note
+ continue behavior.)

## Adapter structure (fetch/parse split, mirroring Yahoo)

```
src/adapters/sources/reddit/
  mod.rs       RedditSource { client, client_id, client_secret, user_agent,
                              token: tokio::sync::RwLock<Option<CachedToken>> }
                 - new(client_id, client_secret) -> Self
                 - impl SocialDataSource: kind() -> Reddit
                     fetch(): ensure_token() → GET search → response::parse_posts
  auth.rs      CachedToken { bearer: SecretString, expires_at: DateTime<Utc> }
                 - is_expired(now) (pure, unit-tested; 60s skew margin)
                 - request_token(client, id, secret, ua) → CachedToken  (the OAuth call)
                 - ensure_token(&self): return a valid bearer, refreshing if expired
  response.rs  serde DTOs + parse_posts(body, limit) -> Result<Vec<SocialPost>, DomainError>
                 - pure, deterministic, fully unit-tested with fixture JSON
```

The HTTP calls (token POST, search GET) are the only impure parts. Token
caching uses `RwLock` interior mutability because the port's `fetch(&self, …)`
is `&self`; one token is reused across a `scan_watchlist` sweep.

## Data flow — query & mapping

Request (one call, curated finance subs):

```
GET https://oauth.reddit.com/r/wallstreetbets+stocks+options+investing+StockMarket/search
      ?q=$TICKER&restrict_sr=1&sort=new&type=link&limit={min(limit,100)}&raw_json=1
Headers: Authorization: bearer <token>,  User-Agent: <required UA>
```

- `restrict_sr=1` keeps results within the listed subs; `sort=new` for recency;
  `type=link` = submissions; `raw_json=1` disables HTML entity escaping.
- `q=$TICKER` (dollar-prefixed cashtag) + subreddit restriction reduces
  false positives. *(Open item A — subreddit set.)*

Response is a Listing: `{ kind: "Listing", data: { children: [ { kind: "t3",
data: {...} } ] } }`. Map each `child.data` → `SocialPost`:

| Field | Rule |
|---|---|
| `id` | `name` (e.g. `t3_abc`) → fallback `id` |
| `author` | `author` (may be `"[deleted]"` — kept as-is) |
| `text` | `title`, plus `"\n" + selftext` when `selftext` is non-empty, truncated to `PostText`'s 10k-char cap before `PostText::parse`; skip the post if the parsed text is empty |
| `created_at` | `created_utc` (f64 seconds) → `DateTime<Utc>` |
| `engagement` | `score.max(0) as u32` (Reddit scores can be negative) |
| `source` | `SourceKind::Reddit` |

*(Open item B — `text` = title+selftext, `engagement` = clamped score.)*
`limit` is honored via the request param and a final `take(limit)`.

## Social-side DI (parallel to the market DI)

`build_sources` is currently hardcoded to mocks. Replace with injection:

- `application::analyze(req, social_sources: &[Box<dyn SocialDataSource>],
  market_source: Option<&dyn MarketDataSource>)` — filters `social_sources` by
  `req.enabled_sources` via `source.kind()`, fetches the enabled ones.
- Signatures updated the same way through `cli::run::analyze` and the three
  `mcp::tools` run functions; `run_list_sources` reports the actually-wired
  social sources (so an agent can see whether Reddit is live).
- Composition roots build the source list:
  - **production:** `[MockXSource, MockBlueskySource]` + `RedditSource` when
    both creds are present.
  - **tests:** `[MockRedditSource, MockXSource, MockBlueskySource]` (assertions
    unchanged — mock data is identical).
- The MCP server holds the social source `Vec` (built once at `serve()`),
  passing `&self.social` into the tools alongside `&self.market`.

Two injected params (social slice + optional market) is the full set of IO
ports for now; a `Deps` struct is a future consolidation if the analyzer
(`PostAnalyzer`) is later injected too (YAGNI now).

## Error handling

All failures → `DomainError::SourceFailure { name: "reddit", message }`:

- token request non-2xx / transport / timeout → `"token request failed: …"`
- 401 on token or search → `"unauthorized — check client id/secret"`
- 429 → `"rate limited (HTTP 429)"` (surfacing `X-Ratelimit-Reset` is a tracked follow-up)
- search non-2xx / malformed JSON → `SourceFailure`
- **No `unwrap`/`expect` on network data.**

The application layer already catches a failed source into a note and continues
with the remaining sources.

## Testing

Hermetic by default — `cargo test` never touches the network.

- **Pure `parse_posts` unit tests** (fixture Listing JSON): happy multi-post,
  `[deleted]` author, empty `selftext` (title only), negative `score` → `0`,
  over-10k `text` truncation, empty `children`, missing optional fields.
- **`CachedToken::is_expired`** unit-tested with fixed `now` values (incl. the
  60s skew margin).
- **OAuth / HTTP paths** are not unit-testable without live creds → one
  `#[ignore]`d live test reading `OPENINTEL_REDDIT_CLIENT_ID`/`SECRET` from env
  (asserts a non-empty search returns ≥0 posts with valid fields); skipped in CI.
- **Social DI:** existing tests inject `[MockReddit, MockX, MockBluesky]`
  (assertions unchanged); add a test that enabling reddit with no wired source
  produces the "not configured" note and still returns other sources.

## Config change

- `Credentials`: replace `reddit_token` with `reddit_client_id` +
  `reddit_client_secret` (`Option<SecretString>`); update `from_env` and the
  secret tests.
- README: Reddit setup (register app → env vars), that it's the richest signal,
  and that keyless Reddit is not available.

## Non-goals (YAGNI)

- Comment fetching (submissions only).
- Configurable subreddits / query tuning (curated set is fixed in v1).
- OS keychain secret storage (documented future opt-in).
- X (paid) and Bluesky (keyless, sparse) adapters — separate future cycles.
- Retry/backoff beyond graceful degradation.

## Open items for review

- **A. Subreddit set** — `wallstreetbets + stocks + options + investing +
  StockMarket`. Good default?
- **B. Mapping** — `text` = title (+ selftext), `engagement` = `score.max(0)`.
  Reasonable?
- **C. User-Agent** — hardcoded `rust:openintel:v{version} (by /u/openintel)`
  vs. an `OPENINTEL_REDDIT_USER_AGENT` override. Recommendation: hardcoded v1.
- **Secret storage** — env-var (recommended) vs. OS keychain now.

## Files

**Create**
- `src/adapters/sources/reddit/mod.rs`
- `src/adapters/sources/reddit/auth.rs`
- `src/adapters/sources/reddit/response.rs`

**Modify**
- `src/adapters/sources/mod.rs` (`pub mod reddit;`)
- `src/config/secrets.rs` (client id/secret)
- `src/application/analyze.rs` (inject social sources; drop hardcoded `build_sources`)
- `src/cli/run.rs`, `src/main.rs` (build + inject the social list)
- `src/mcp/tools.rs`, `src/mcp/server.rs` (thread + own the social list)
- `tests/analyze_flow.rs` (inject mock social list)
- `README.md` (Reddit setup + secret guidance)
