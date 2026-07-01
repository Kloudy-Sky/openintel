# Yahoo Finance Market Adapter — Design

**Date:** 2026-06-30
**Status:** Approved

## Goal

Replace the hardcoded `MockMarketSource` in the analysis path with a real,
**keyless Yahoo Finance** `MarketDataSource`, so `openintel analyze` (and the
MCP tools) show live price action with zero configuration — while the test
suite stays fully hermetic (no network).

## Why Yahoo, keyless

- **Zero-config UX.** No API key, no signup — a fresh clone runs `cargo run --
  analyze AAPL` against real market data immediately. Matches the project's
  "simplest possible UX" priority.
- **Fills every engine-critical field.** The engine's math depends only on
  `last_price`, `previous_close`, `volume`, `avg_volume` (all available) plus
  optional `iv_rank` (crowding renormalizes cleanly when absent — already
  tested).
- **Trade-off accepted:** the v8 chart endpoint is unofficial (ToS-gray, can
  change without notice). Failures degrade gracefully to a social-only report
  (already handled in the application layer). Adding a keyed/official provider
  later is a new struct + `impl MarketDataSource` behind the same port.

## Architecture

Hexagonal composition-root pattern. The two real entry points construct the
concrete adapter; every layer below depends only on the `MarketDataSource`
port.

```
main.rs (Analyze branch) ─┐
mcp::server::serve()      ─┴─► build YahooMarketSource ──► inject &dyn MarketDataSource
                                                              │
   cli::run::analyze(config, market) ─┐                       │
   mcp::tools::run_{analyze,scan,compare,list_sources}(…, market) ─┴─► application::analyze(req, market)
                                                                          └─► market.snapshot(&ticker)
```

Tests construct `MockMarketSource` and inject it the same way — no network.

### New adapter (fetch/parse split)

```
src/adapters/market/yahoo/
  mod.rs        YahooMarketSource { client: reqwest::Client }
                  - new() -> Result<Self, DomainError>   (client w/ 10s timeout + UA)
                  - impl MarketDataSource:
                      name() -> "yahoo"
                      snapshot(&ticker): reads clock at edge, fetches, delegates to parse
  response.rs   serde DTOs + parse_snapshot(body, &ticker, fetched_at) + realized_vol()
                  - pure, deterministic, fully unit-tested with fixture JSON
```

The HTTP call is the only impure part and lives in `mod.rs`. **All mapping and
math is a pure `parse_snapshot` function** fed captured JSON in tests.

## Data flow — endpoint & field mapping

`GET https://query1.finance.yahoo.com/v8/finance/chart/{SYMBOL}?range=3mo&interval=1d`
(one request; a browser-ish `User-Agent` header to avoid default-agent 429s).

Response shape (relevant fields, serde `rename_all = "camelCase"`):

```
chart.error                       -> Option<{ code, description }>
chart.result[0].meta.regularMarketPrice   -> Option<f64>
chart.result[0].meta.chartPreviousClose   -> Option<f64>
chart.result[0].meta.regularMarketVolume  -> Option<u64>
chart.result[0].meta.regularMarketTime    -> Option<i64>   (unix seconds)
chart.result[0].timestamp                  -> Option<Vec<i64>>
chart.result[0].indicators.quote[0].close  -> Vec<Option<f64>>   (null-padded)
chart.result[0].indicators.quote[0].volume -> Vec<Option<u64>>   (null-padded)
```

Mapping to `MarketSnapshot` (closes/volumes = the series with nulls dropped,
order preserved):

| Field | Rule | Fallback |
|---|---|---|
| `ticker` | the requested `Ticker` (clone) | — |
| `last_price` | `meta.regularMarketPrice` | last non-null close |
| `previous_close` | 2nd-to-last non-null close | `meta.chartPreviousClose` → else `SourceFailure` |
| `volume` | `meta.regularMarketVolume` | last non-null volume → else `0` |
| `avg_volume` | mean of non-null volumes (round to u64) | `0` if none |
| `realized_vol` | `√252 · sample_stdev(ln cₜ/cₜ₋₁)` over non-null closes | `None` if `< 20` returns |
| `put_call_ratio` | `None` (not in this endpoint) | — |
| `iv_rank` | `None` (not in this endpoint) | — |
| `as_of` | `meta.regularMarketTime` (unix→UTC) | last `timestamp` → else `fetched_at` |

Rationale for `previous_close` from the series' 2nd-to-last close: during
market hours the last daily bar is the in-progress session and
`regularMarketPrice` is live, so the prior *completed* close is the correct
daily-change reference. `volume`/`avg_volume` of `0` are safe — the engine
already guards `avg_volume == 0` (rvol omitted + note). `last_price` and
`previous_close` are the only hard requirements; their absence is a
`SourceFailure`.

`fetched_at` is captured by `snapshot()` via `Utc::now()` *before* calling the
pure `parse_snapshot`, keeping the clock at the edge and `parse_snapshot`
deterministic.

## Dependency injection — exact changes

**Signatures (thread `market: &dyn MarketDataSource`):**
- `application::analyze(req: &AnalysisRequest, market: &dyn MarketDataSource)`
- `cli::run::analyze(config: &AppConfig, market: &dyn MarketDataSource)`
- `mcp::tools::run_analyze(args, market: &dyn MarketDataSource)`
- `mcp::tools::run_scan(args, market: &dyn MarketDataSource)`
- `mcp::tools::run_compare(args, market: &dyn MarketDataSource)`
- `mcp::tools::run_list_sources(market: &dyn MarketDataSource)` (reports `market.name()`)

In `run_scan`/`run_compare` the injected `&dyn MarketDataSource` is `Send +
Sync` and `Copy` (a reference), so each `join_all` closure captures it by copy
and borrows the one shared source for the duration of the await — no `Arc`
needed.

**Composition roots (build the concrete adapter):**
- `main.rs` `Command::Analyze`: `let market = YahooMarketSource::new()?;` then
  `analyze(&config, &market)`. A `new()` error prints `error: …` and returns
  `ExitCode::FAILURE`.
- `mcp::server::serve()`: `let market = YahooMarketSource::new()?;` then
  `OpenIntelServer::new(market)`.

**MCP server holds the source:**
- `OpenIntelServer { tool_router, market: YahooMarketSource }`.
  `YahooMarketSource` derives `Clone` (`reqwest::Client` is `Clone`), so the
  server's existing `#[derive(Clone)]` still holds.
- `OpenIntelServer::new(market: YahooMarketSource)`; drop the `Default` impl
  (construction now needs a source; `new` takes an arg so the
  `new_without_default` lint does not fire).
- Each tool method passes `&self.market` (coerces to `&dyn`).

Social sources stay mock-wired via `build_sources` (unchanged) — real social
adapters are a later cycle (YAGNI).

## Error handling

All failure modes map to the existing `DomainError::SourceFailure { name:
"yahoo", message }` — no new error variant:

- non-2xx / transport error / timeout → `SourceFailure` (message includes HTTP
  status when present)
- `chart.error` present (unknown symbol, etc.) → `SourceFailure(description)`
- empty `result` / malformed JSON / missing `last_price`/`previous_close` →
  `SourceFailure`
- **No `unwrap`/`expect` on network data.**

The application layer already catches a failed market fetch → pushes a
`"market source failed: …"` note and continues with `market = None` (social-only
report). No changes needed there beyond the injected call.

**Resilience scope (v1):** single attempt, 10s timeout, no retry. Graceful
degradation covers transient failures; a retry/backoff is an easy future add.

## Dependency

Add `reqwest` with rustls (no OpenSSL) + JSON. Resolve the current version
with `cargo add` — do not hand-pin from memory:

```
cargo add reqwest --no-default-features --features rustls-tls,json
```

This yields a `reqwest = { version = "…", default-features = false, features =
["rustls-tls", "json"] }` entry. `rustls-tls` avoids an OpenSSL system
dependency; `json` enables typed deserialization.

## Testing

**Hermetic by default — `cargo test` never touches the network.**

- **Pure `parse_snapshot` unit tests** (fixture JSON as `const` strings in
  `response.rs`):
  - happy path (realistic multi-day AAPL-shaped response) → asserts each mapped
    field
  - `chart.error` JSON (unknown symbol) → `SourceFailure`
  - empty `result` → `SourceFailure`
  - missing `last_price`/`previous_close` → `SourceFailure`
  - null-padded `close`/`volume` arrays → nulls dropped, order preserved
  - `< 20` returns → `realized_vol == None`; `≥ 20` → `Some`
- **`realized_vol` numeric test** with a known close series (assert against a
  hand-computed annualized value within `1e-9`).
- **Update existing tests to inject `MockMarketSource`:**
  `application/analyze.rs` (2), `cli/run.rs` (4), `mcp/tools.rs` (~6, incl.
  `list_sources` now asserting `"mock-market"`), `tests/analyze_flow.rs` (3,
  importing `openintel::adapters::market::mock_market::MockMarketSource`).
- **One `#[ignore]`d live smoke test** hitting Yahoo for `AAPL` (asserts
  `last_price > 0`), run via `cargo test -- --ignored`. Not in the CI default.

## Docs

- `README.md`: market is now **real (Yahoo, keyless)**; social still mock.
  Note that market requires network — offline runs degrade to a social-only
  report. Update the stale "Extending → market source: Select it in `cli::run`"
  note to describe the DI composition root.

## Non-goals (YAGNI)

- Options-chain fetch (`put_call_ratio` / `iv_rank` stay `None`).
- Caching, retries/backoff, rate-limit handling.
- Multi-provider runtime selection (Yahoo only in production; mock via DI for
  tests).

## Files

**Create**
- `src/adapters/market/yahoo/mod.rs`
- `src/adapters/market/yahoo/response.rs`

**Modify**
- `Cargo.toml` (add `reqwest`)
- `src/adapters/market/mod.rs` (`pub mod yahoo;`)
- `src/application/analyze.rs` (DI signature + body + tests)
- `src/cli/run.rs` (DI signature + body + tests)
- `src/main.rs` (build Yahoo, pass to `analyze`)
- `src/mcp/tools.rs` (4 fn signatures + tests)
- `src/mcp/server.rs` (server holds source; `serve()` builds it; drop `Default`)
- `tests/analyze_flow.rs` (inject `MockMarketSource`)
- `README.md`
