# X Pulse — Influencer Event Feed (Credit-Aware, Opt-In) — Design

**Date:** 2026-07-16
**Status:** Draft — awaiting user review

## Goal

Surface posts from **specific high-impact X accounts** about a ticker — market-moving
events (a POTUS tariff post, a CEO announcement, an activist thread), not another
sentiment average. X's pay-per-use API ($0.005/post read, verified 2026-07 from
docs.x.com) makes commodity mention-scanning bad value (Bluesky covers that free);
the differentiated, cheap, high-signal query is **author-filtered search** —
typically 0–5 posts, costing pennies, each one a catalyst candidate.

**Explicitly a new surface, not a fifth sentiment source**: pulse posts are events
for the consuming agent (or human) to reason about. They never enter the fusion
engine's averaging. `SourceKind::X` does **not** return.

## Product shape

### CLI

```text
openintel pulse NVDA --accounts jensenhuang,elonmusk --hours 24 --limit 20
```

- `--accounts a,b,c` — X handles to listen to (no `@`). Omitted → the shipped
  default macro list.
- `--hours N` — lookback window, default 24, clamped 1..=168.
- `--limit N` — max posts, default 20, clamped 1..=100.
- `--format table|json` — like `analyze`.

Table output:

```text
=== OpenIntel X Pulse — NVDA ===
window: last 24h · accounts: jensenhuang, elonmusk, realDonaldTrump, WhiteHouse
generated: 2026-07-16T22:40:00Z

⚡ 2 post(s)

  [3h ago] @jensenhuang (eng 48210)
    Blackwell Ultra is now shipping at scale. The next decade of computing…

  [11h ago] @realDonaldTrump (eng 191034)
    Chips made in America will be TAXED at ZERO. Foreign chips, big tariffs!

cost: 2 posts read (≈ $0.01 at $0.005/read; X dedupes re-reads for 24h)

Not financial advice. …existing disclaimer…
```

Zero posts is a **successful quiet result** (exit 0, "no posts from these accounts
in the window"). Unconfigured X → clear error + `run: openintel setup x`, exit 1.

### MCP tool: `x_pulse`

Input `{ ticker, accounts?: string[], hours_back?: u32 (default 24), limit?: usize (default 20) }`
→ the `PulseReport` as JSON plus the disclaimer.

**The curation contract lives in the tool description** (this is the guided UX —
the agent does the research, the human approves the spend):

> Fetch recent posts about `ticker` from specific high-impact X accounts (paid
> API: ~$0.005 per post read). Before calling: research which accounts actually
> matter for this ticker — CEO/founder, major institutional holders or activist
> funds, respected sector journalists, and market-moving macro figures — then
> propose the account list and estimated max cost (`limit × $0.005`) to the user
> and get their confirmation. Omit `accounts` only if the user asks for the
> default macro list. Returned posts are catalyst events — reason about them
> directly; do not treat them as a sentiment sample.

### Default macro accounts

```rust
pub const DEFAULT_PULSE_ACCOUNTS: [&str; 4] =
    ["realDonaldTrump", "WhiteHouse", "elonmusk", "federalreserve"];
```

Small and defensible: heads of state/government move indices and sectors; Musk
moves specific tickers; the Fed moves everything. Per-call override is the primary
path — this list is only the no-arguments fallback.

## Architecture (hexagonal, mirrors existing patterns)

### Domain

- `domain/entities/pulse.rs` (new):
  - `PulsePost { id: String, author: String, text: PostText, created_at: DateTime<Utc>, engagement: u32 }`
    (reuses `PostText`; deliberately NOT `SocialPost` — no `SourceKind`).
  - `PulseReport { ticker: String, accounts: Vec<String>, hours_back: u32, posts: Vec<PulsePost>, posts_read: u32, estimated_cost_usd: f64, generated_at: DateTime<Utc> }`
    — all `Serialize`.
- `domain/ports/influencer_feed.rs` (new):

```rust
#[async_trait]
pub trait InfluencerFeed: Send + Sync {
    async fn pulse(
        &self,
        ticker: &Ticker,
        accounts: &[String],
        hours_back: u32,
        limit: usize,
    ) -> Result<Vec<PulsePost>, DomainError>;
}
```

### Application

- `application/pulse.rs` (new): validate ticker (`Ticker::parse`), clamp
  `hours_back` 1..=168 and `limit` 1..=100, normalize accounts (strip leading `@`,
  drop empties; empty list after normalize → `DEFAULT_PULSE_ACCOUNTS`), call the
  feed, assemble `PulseReport` (`posts_read = posts.len()`,
  `estimated_cost_usd = posts_read as f64 * X_COST_PER_READ_USD`). Clock injected
  at the edge per house pattern.
- `pub const X_COST_PER_READ_USD: f64 = 0.005;` — one constant, commented with
  source + as-of date (docs.x.com pricing, 2026-02 pay-per-use launch).

### Adapter — `adapters/sources/x/` (mod.rs + response.rs)

- `XPulseSource::new(bearer: SecretString) -> Result<Self, DomainError>` — reqwest
  client, 10 s timeout, `rust:openintel:v{version}` UA. Implements `InfluencerFeed`.
- Request: `GET https://api.x.com/2/tweets/search/recent` with
  `Authorization: Bearer` (`.expose_secret()` at that one site) and query params
  via `Url::query_pairs_mut()` (reqwest 0.13.4 gating, as everywhere):
  - `query` = `` $TICKER (from:a OR from:b …) -is:retweet `` — built by a pure,
    unit-tested `build_query(ticker, accounts)`.
    *Contingency (decided at live test with the user's token):* if pay-per-use
    rejects the `$` cashtag operator (HTTP 400 naming the operator), switch to
    the bare ticker keyword — one-line change in `build_query`, tests updated.
  - `start_time` = now − hours_back, RFC 3339.
  - `max_results` = `clamp(limit, 10, 100)` (API minimum is 10; parser truncates
    to the requested `limit` so the report never over-shows).
  - `tweet.fields=created_at,public_metrics`, `expansions=author_id`,
    `user.fields=username`.
- `response.rs` — pure `parse_posts(body, limit, fetched_at)`: join
  `includes.users` by `author_id` for the handle (fallback `"[unknown]"`);
  `created_at` RFC 3339 → fallback `fetched_at`; engagement =
  like + retweet + reply counts (defaults 0, saturating, per Bluesky precedent);
  skip empty text / missing id; `#[serde(default)]` throughout; malformed JSON →
  `SourceFailure`.
- Error mapping (`name: "x"`):
  - 401 → `"unauthorized — check bearer token"`
  - 403 → `"forbidden — check API access and credit balance"`
  - 429 → `"rate limited (HTTP 429) — resets at {UTC time}"`, reset parsed from
    the `x-rate-limit-reset` header (unix seconds) when present, else the plain
    429 message. **Fail fast** — a pulse is one request; no wait/retry in v1.
  - other non-2xx → `"search HTTP {status}"`. Response bodies never echoed.

### Credentials & setup

- `Credentials.x_bearer: Option<SecretString>` returns — env `OPENINTEL_X_BEARER`,
  keychain key `OPENINTEL_X_BEARER`, resolved by `Credentials::load` like the rest.
- `openintel setup x` — the interactive flow gets its first **single-credential**
  source. `SourceSpec.second_*` fields become `Option`s (Reddit/Bluesky pass
  `Some`, X passes `None`); `plan()`'s pair logic applies only to pair sources —
  single-credential mode selection is just set/unset (Guide / Verify; Partial
  cannot occur). Non-TTY behavior for reddit/bluesky is byte-identical.
- **Verification costs money and says so first**: after the guide + hidden bearer
  prompt, before probing:
  `Verifying will read up to 10 posts from X (≈ $0.05). Proceed? [Y/n]` —
  decline → nothing saved, exit 1. Probe = `pulse(AAPL, DEFAULT_PULSE_ACCOUNTS,
  24h, limit 10)`. A 0-post result still verifies (auth + query succeeded).
  Non-TTY with `OPENINTEL_X_BEARER` set verifies directly (cost documented in
  README; CI users know what they're doing).

### Composition & wiring

- `main.rs`: `Command::Pulse(args)` arm — builds `XPulseSource` from resolved
  creds (error + setup hint when absent), calls `application::pulse`, renders.
- `mcp/server.rs` / `mcp/tools.rs`: server holds `Option<Arc<XPulseSource>>`
  (None when unconfigured → tool returns the setup-hint error); `x_pulse` tool
  as above. `analyze`/`scan`/`compare`/`list_sources` are untouched.
- `list_sources` gains nothing in v1 — pulse is not a `SocialDataSource`
  (revisit if the tool inventory grows).

## Cost & rate-limit posture (the user-set requirements)

- **Opt-in only**: nothing calls X except an explicit `pulse` invocation. No
  default-run spend, ever.
- **Cost transparency**: every report carries `posts_read` + `estimated_cost_usd`
  and the table renders the cost line; the MCP description forces a pre-call
  estimate + user confirmation; setup states its probe cost.
- **Rate limits**: header-aware fail-fast with reset time surfaced.
  `--wait-for-rate-limit` is deliberately deferred (a pulse is a single request;
  waiting matters when multi-request features arrive).

## Testing (hermetic)

- `build_query`: single/multi accounts, `@`-stripping, cashtag + `-is:retweet`
  presence, OR-chain shape.
- Parser: happy path with `includes.users` join, missing author → `[unknown]`,
  missing `created_at` → `fetched_at`, engagement summing/saturation, empty-text
  skip, limit truncation below `max_results`, malformed JSON, empty `data`.
- Application: clamps (hours 0→1, 200→168; limit 0→1, 500→100), `@`-normalize,
  empty accounts → defaults, cost math (`3 posts → 0.015`).
- Setup: single-credential mode selection, X guide copy invariants (developer
  console URL, cost warning, "never stores"), cost-confirm decline saves nothing.
- Args: `pulse` parsing (accounts comma-split, defaults), setup x parses.
- One `#[ignore]`d live test (`x_live_pulse`) — run manually with the user's
  bearer; also the cashtag-operator contingency check.
- Render: table contains window, accounts, per-post lines, cost line, disclaimer.

## Docs

- README: new "X Pulse (optional, paid)" section — what it is (catalyst feed,
  not sentiment), the cost math ($0.005/read; default limit 20 → ≤ 10¢/call;
  24 h dedup), `setup x`, curation philosophy (the agent researches accounts,
  you approve). Quickstart line mentions it after the free sources.
- `.env.example`: `OPENINTEL_X_BEARER` returns with a "paid API — see README
  cost notes" comment.
- SECURITY.md: no changes needed (bearer is a `SecretString` through the
  existing env/keychain machinery).

## Non-goals (YAGNI)

- No `SourceKind::X`, no fusion/`analyze` integration, no sentiment averaging of
  pulse posts (the consuming agent reasons about events; a future fusion note is
  a separate decision).
- No mention-scan mode (commodity X scanning is bad value; additive later).
- No `--wait-for-rate-limit`, no pagination, no streaming/webhooks, no scheduled
  monitoring.
- No account-list persistence (config files) — per-call + defaults only; the
  agent is the curator.
- No options/risk math (that's the next feature, `risk_frame`).

## Files

**Create**
- `src/domain/entities/pulse.rs`, `src/domain/ports/influencer_feed.rs`
- `src/application/pulse.rs`
- `src/adapters/sources/x/{mod.rs,response.rs}`
- `src/cli/pulse.rs` — the CLI leaf: table/json render + orchestration
  (mirrors how `setup.rs` is a leaf; `cli/run.rs` stays analyze-only)
- this spec

**Modify**
- `src/domain/{entities,ports}/mod.rs`-equivalents (module registrations)
- `src/application/mod.rs`
- `src/config/secrets.rs` (+`x_bearer`)
- `src/cli/args.rs` (`Pulse(PulseArgs)`, `SetupSource::X`), `src/cli/setup.rs`
  (single-credential flow)
- `src/main.rs`, `src/mcp/server.rs`, `src/mcp/tools.rs`
- `README.md`, `.env.example`
