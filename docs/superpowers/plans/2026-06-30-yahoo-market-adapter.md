# Yahoo Finance Market Adapter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the hardcoded `MockMarketSource` in the analysis path with a real, keyless Yahoo Finance `MarketDataSource`, injected via dependency injection so tests stay hermetic.

**Architecture:** A pure `parse_snapshot` function maps the Yahoo v8 chart JSON to `MarketSnapshot`; a thin `YahooMarketSource` does the one HTTP call and delegates to it. `analyze()` and the MCP tools take an injected `&dyn MarketDataSource`; the two real entry points (`main.rs` analyze branch, `mcp::serve()`) construct the concrete `YahooMarketSource`, while tests inject `MockMarketSource`.

**Tech Stack:** Rust, tokio, reqwest (rustls), serde_json, chrono.

## Global Constraints

- Market provider: **Yahoo Finance v8 chart, keyless** — `GET https://query1.finance.yahoo.com/v8/finance/chart/{SYMBOL}?range=3mo&interval=1d`.
- **`cargo test` must never touch the network.** All parsing logic is unit-tested via fixture JSON; the only live test is `#[ignore]`d.
- All adapter failures map to the existing `DomainError::SourceFailure { name: "yahoo".into(), message }` — **do not add a new error variant**.
- **No `unwrap`/`expect` on network data** (response body, HTTP status, JSON fields).
- Add `reqwest` via `cargo add` (do not hand-pin a version from memory); rustls TLS (no OpenSSL).
- DI composition root: entry points (`main.rs`, `mcp::serve()`) build the concrete adapter; every layer below takes `&dyn MarketDataSource`.
- YAGNI: `put_call_ratio`/`iv_rank` stay `None`; no caching, retries, or multi-provider selection.
- Clock at the edge: `snapshot()` reads `Utc::now()` and passes it to the pure `parse_snapshot`; `parse_snapshot` is deterministic.
- After each task: `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` all clean.

---

## File Structure

**Create**
- `src/adapters/market/yahoo/mod.rs` — `YahooMarketSource` (HTTP shell + `impl MarketDataSource`).
- `src/adapters/market/yahoo/response.rs` — serde DTOs + pure `parse_snapshot` + vol math + unit tests.

**Modify**
- `Cargo.toml` — add `reqwest`.
- `src/adapters/market/mod.rs` — `pub mod yahoo;`.
- `src/application/analyze.rs` — DI signature + body + tests.
- `src/cli/run.rs` — DI signature + body + tests.
- `src/main.rs` — build Yahoo, pass to `analyze`.
- `src/mcp/tools.rs` — 4 fn signatures + tests.
- `src/mcp/server.rs` — server owns source; `serve()` builds it; drop `Default`.
- `tests/analyze_flow.rs` — inject `MockMarketSource`.
- `README.md` — market now real (Yahoo); DI note.

---

## Task 1: Pure Yahoo response parser

**Files:**
- Create: `src/adapters/market/yahoo/mod.rs` (this task: only `mod response;`)
- Create: `src/adapters/market/yahoo/response.rs`
- Modify: `src/adapters/market/mod.rs`
- Test: unit tests inside `src/adapters/market/yahoo/response.rs`

**Interfaces:**
- Consumes: `MarketSnapshot` (`src/domain/entities/market_snapshot.rs`), `Ticker` (`::ticker`), `DomainError::SourceFailure { name, message }` (`src/domain/error.rs`).
- Produces (all `pub(crate)`):
  - `parse_snapshot(body: &str, ticker: &Ticker, fetched_at: DateTime<Utc>) -> Result<MarketSnapshot, DomainError>`
  - `sample_stdev(xs: &[f64]) -> Option<f64>`
  - `log_returns(closes: &[f64]) -> Vec<f64>`
  - `realized_vol(closes: &[f64], min_returns: usize) -> Option<f64>`

- [ ] **Step 1: Register the module tree**

Modify `src/adapters/market/mod.rs` to read:

```rust
pub mod mock_market;
pub mod yahoo;
```

Create `src/adapters/market/yahoo/mod.rs` with only:

```rust
mod response;
```

- [ ] **Step 2: Write the failing tests**

Create `src/adapters/market/yahoo/response.rs` with the test module first (the fixtures + assertions). Paste this whole test module at the bottom of the file:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn tkr() -> Ticker {
        Ticker::parse("AAPL").unwrap()
    }
    fn at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 30, 0, 0, 0).unwrap()
    }

    // 3 daily bars; last live price in meta differs from series last close.
    const HAPPY: &str = r#"{"chart":{"result":[{
        "meta":{"regularMarketPrice":192.5,"chartPreviousClose":170.0,
                "regularMarketVolume":95000000,"regularMarketTime":1782504000},
        "timestamp":[1782327600,1782414000,1782500400],
        "indicators":{"quote":[{"close":[185.0,188.0,191.0],"volume":[50000000,60000000,95000000]}]}
    }],"error":null}}"#;

    const NULL_PADDED: &str = r#"{"chart":{"result":[{
        "meta":{"regularMarketPrice":10.0,"regularMarketVolume":30},
        "timestamp":[1,2,3,4],
        "indicators":{"quote":[{"close":[null,8.0,null,9.0],"volume":[null,10,null,20]}]}
    }],"error":null}}"#;

    const ERROR_BODY: &str = r#"{"chart":{"result":null,"error":{"code":"Not Found","description":"No data found, symbol may be delisted"}}}"#;

    const EMPTY_RESULT: &str = r#"{"chart":{"result":[],"error":null}}"#;

    const NO_PRICE: &str = r#"{"chart":{"result":[{
        "meta":{},"timestamp":[],"indicators":{"quote":[{"close":[],"volume":[]}]}
    }],"error":null}}"#;

    #[test]
    fn happy_path_maps_all_fields() {
        let s = parse_snapshot(HAPPY, &tkr(), at()).unwrap();
        assert_eq!(s.ticker.as_str(), "AAPL");
        assert_eq!(s.last_price, 192.5); // from meta.regularMarketPrice
        assert_eq!(s.previous_close, 188.0); // 2nd-to-last non-null close
        assert_eq!(s.volume, 95000000); // meta.regularMarketVolume
        assert_eq!(s.avg_volume, 68333333); // round((50+60+95)e6 / 3)
        assert_eq!(s.realized_vol, None); // only 2 returns < 20
        assert_eq!(s.put_call_ratio, None);
        assert_eq!(s.iv_rank, None);
        assert_eq!(s.as_of, Utc.timestamp_opt(1782504000, 0).single().unwrap());
    }

    #[test]
    fn null_padding_is_dropped_order_preserved() {
        let s = parse_snapshot(NULL_PADDED, &tkr(), at()).unwrap();
        // non-null closes = [8.0, 9.0] -> previous_close = 8.0
        assert_eq!(s.previous_close, 8.0);
        assert_eq!(s.last_price, 10.0); // meta price
        // non-null volumes = [10, 20] -> avg = 15
        assert_eq!(s.avg_volume, 15);
        assert_eq!(s.volume, 30); // meta volume
        // no meta time, no fallback timestamp path returns last timestamp = 4
        assert_eq!(s.as_of, Utc.timestamp_opt(4, 0).single().unwrap());
    }

    #[test]
    fn chart_error_is_source_failure() {
        let err = parse_snapshot(ERROR_BODY, &tkr(), at()).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("yahoo"), "got {msg}");
        assert!(msg.contains("delisted"), "got {msg}");
    }

    #[test]
    fn empty_result_is_source_failure() {
        assert!(parse_snapshot(EMPTY_RESULT, &tkr(), at()).is_err());
    }

    #[test]
    fn missing_price_is_source_failure() {
        assert!(parse_snapshot(NO_PRICE, &tkr(), at()).is_err());
    }

    #[test]
    fn malformed_json_is_source_failure() {
        assert!(parse_snapshot("not json", &tkr(), at()).is_err());
    }

    #[test]
    fn sample_stdev_math() {
        assert_eq!(sample_stdev(&[1.0, 2.0, 3.0]), Some(1.0)); // var=1, stdev=1
        assert_eq!(sample_stdev(&[2.0, 2.0]), Some(0.0));
        assert_eq!(sample_stdev(&[5.0]), None);
        assert_eq!(sample_stdev(&[]), None);
    }

    #[test]
    fn log_returns_len_and_values() {
        let r = log_returns(&[100.0, 110.0, 121.0]);
        assert_eq!(r.len(), 2);
        assert!((r[0] - 1.1f64.ln()).abs() < 1e-12);
        assert!((r[1] - 1.1f64.ln()).abs() < 1e-12);
    }

    #[test]
    fn realized_vol_gate_and_value() {
        // gate: fewer than min_returns -> None
        assert_eq!(realized_vol(&[100.0, 110.0], 20), None);
        // equal returns -> stdev 0 -> Some(0.0)
        assert_eq!(realized_vol(&[100.0, 110.0, 121.0], 2), Some(0.0));
        // known value: closes [100,110,90], min 2 -> ~3.3223 (annualized, sqrt(252))
        let v = realized_vol(&[100.0, 110.0, 90.0], 2).unwrap();
        assert!((v - 3.3223).abs() < 1e-3, "got {v}");
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p openintel --lib adapters::market::yahoo`
Expected: FAIL to compile — `parse_snapshot`, `sample_stdev`, `log_returns`, `realized_vol` not found.

- [ ] **Step 4: Write the implementation**

Prepend this to `src/adapters/market/yahoo/response.rs` (above the test module):

```rust
use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;

use crate::domain::entities::market_snapshot::MarketSnapshot;
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;

const MIN_RETURNS_FOR_VOL: usize = 20;
const TRADING_DAYS: f64 = 252.0;

#[derive(Debug, Deserialize)]
struct ChartResponse {
    chart: Chart,
}

#[derive(Debug, Deserialize)]
struct Chart {
    #[serde(default)]
    result: Option<Vec<ChartResult>>,
    #[serde(default)]
    error: Option<YahooError>,
}

#[derive(Debug, Deserialize)]
struct YahooError {
    #[serde(default)]
    code: String,
    #[serde(default)]
    description: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChartResult {
    meta: Meta,
    #[serde(default)]
    timestamp: Option<Vec<i64>>,
    indicators: Indicators,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Meta {
    #[serde(default)]
    regular_market_price: Option<f64>,
    #[serde(default)]
    chart_previous_close: Option<f64>,
    #[serde(default)]
    regular_market_volume: Option<u64>,
    #[serde(default)]
    regular_market_time: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct Indicators {
    quote: Vec<Quote>,
}

#[derive(Debug, Deserialize)]
struct Quote {
    #[serde(default)]
    close: Vec<Option<f64>>,
    #[serde(default)]
    volume: Vec<Option<u64>>,
}

fn fail(message: impl Into<String>) -> DomainError {
    DomainError::SourceFailure {
        name: "yahoo".into(),
        message: message.into(),
    }
}

pub(crate) fn sample_stdev(xs: &[f64]) -> Option<f64> {
    if xs.len() < 2 {
        return None;
    }
    let n = xs.len() as f64;
    let mean = xs.iter().sum::<f64>() / n;
    let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0);
    Some(var.sqrt())
}

pub(crate) fn log_returns(closes: &[f64]) -> Vec<f64> {
    closes.windows(2).map(|w| (w[1] / w[0]).ln()).collect()
}

pub(crate) fn realized_vol(closes: &[f64], min_returns: usize) -> Option<f64> {
    let returns = log_returns(closes);
    if returns.len() < min_returns {
        return None;
    }
    sample_stdev(&returns).map(|s| s * TRADING_DAYS.sqrt())
}

pub(crate) fn parse_snapshot(
    body: &str,
    ticker: &Ticker,
    fetched_at: DateTime<Utc>,
) -> Result<MarketSnapshot, DomainError> {
    let resp: ChartResponse =
        serde_json::from_str(body).map_err(|e| fail(format!("malformed response: {e}")))?;

    if let Some(err) = resp.chart.error {
        return Err(fail(format!("{}: {}", err.code, err.description)));
    }

    let result = resp
        .chart
        .result
        .and_then(|mut r| (!r.is_empty()).then(|| r.remove(0)))
        .ok_or_else(|| fail("empty result"))?;

    let quote = result
        .indicators
        .quote
        .into_iter()
        .next()
        .ok_or_else(|| fail("no quote series"))?;

    let closes: Vec<f64> = quote.close.into_iter().flatten().collect();
    let volumes: Vec<u64> = quote.volume.into_iter().flatten().collect();

    let last_price = result
        .meta
        .regular_market_price
        .or_else(|| closes.last().copied())
        .ok_or_else(|| fail("no last price"))?;

    let previous_close = closes
        .len()
        .checked_sub(2)
        .and_then(|i| closes.get(i).copied())
        .or(result.meta.chart_previous_close)
        .ok_or_else(|| fail("no previous close"))?;

    let volume = result
        .meta
        .regular_market_volume
        .or_else(|| volumes.last().copied())
        .unwrap_or(0);

    let avg_volume = if volumes.is_empty() {
        0
    } else {
        (volumes.iter().sum::<u64>() as f64 / volumes.len() as f64).round() as u64
    };

    let realized_vol = realized_vol(&closes, MIN_RETURNS_FOR_VOL);

    let as_of = result
        .meta
        .regular_market_time
        .or_else(|| result.timestamp.as_ref().and_then(|t| t.last().copied()))
        .and_then(|secs| Utc.timestamp_opt(secs, 0).single())
        .unwrap_or(fetched_at);

    Ok(MarketSnapshot {
        ticker: ticker.clone(),
        as_of,
        last_price,
        previous_close,
        volume,
        avg_volume,
        realized_vol,
        put_call_ratio: None,
        iv_rank: None,
    })
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p openintel --lib adapters::market::yahoo`
Expected: PASS (10 tests).

- [ ] **Step 6: Lint + format**

Run: `cargo clippy --all-targets -- -D warnings && cargo fmt`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add src/adapters/market/mod.rs src/adapters/market/yahoo/
git commit -m "feat(market): pure Yahoo chart response parser"
```

---

## Task 2: Yahoo HTTP adapter

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/adapters/market/yahoo/mod.rs`
- Test: unit + `#[ignore]` live test inside `src/adapters/market/yahoo/mod.rs`

**Interfaces:**
- Consumes: `response::parse_snapshot` (Task 1), `MarketDataSource` port (`src/domain/ports/market_data_source.rs`: `fn name(&self) -> &'static str`, `async fn snapshot(&self, ticker: &Ticker) -> Result<MarketSnapshot, DomainError>`).
- Produces: `YahooMarketSource` with `pub fn new() -> Result<Self, DomainError>`; `impl MarketDataSource` (`name()` → `"yahoo"`); `#[derive(Clone)]`.

- [ ] **Step 1: Add the reqwest dependency**

Run: `cargo add reqwest --no-default-features --features rustls-tls`
Expected: `Cargo.toml` gains a `reqwest = { version = "…", default-features = false, features = ["rustls-tls"] }` line.

> Note: we deliberately do NOT enable reqwest's `json` feature — the body is read with `.text()` and parsed by our own `serde_json::from_str` (via `parse_snapshot`) so error messages carry our context. `rustls-tls` avoids an OpenSSL system dependency.

- [ ] **Step 2: Write the failing test**

Add this test module at the bottom of `src/adapters/market/yahoo/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_builds_and_names_yahoo() {
        let src = YahooMarketSource::new().unwrap();
        assert_eq!(src.name(), "yahoo");
    }

    #[tokio::test]
    #[ignore = "hits live Yahoo Finance; run with `cargo test -- --ignored`"]
    async fn live_snapshot_has_positive_prices() {
        let src = YahooMarketSource::new().unwrap();
        let snap = src
            .snapshot(&Ticker::parse("AAPL").unwrap())
            .await
            .unwrap();
        assert!(snap.last_price > 0.0, "last_price = {}", snap.last_price);
        assert!(snap.previous_close > 0.0);
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p openintel --lib adapters::market::yahoo::tests::new_builds_and_names_yahoo`
Expected: FAIL to compile — `YahooMarketSource` not found.

- [ ] **Step 4: Write the implementation**

Replace the contents of `src/adapters/market/yahoo/mod.rs` (keep the test module from Step 2 at the bottom) so the top reads:

```rust
mod response;

use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;

use crate::domain::entities::market_snapshot::MarketSnapshot;
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::ports::market_data_source::MarketDataSource;

const BASE_URL: &str = "https://query1.finance.yahoo.com/v8/finance/chart";
const TIMEOUT_SECS: u64 = 10;

#[derive(Clone)]
pub struct YahooMarketSource {
    client: reqwest::Client,
}

impl YahooMarketSource {
    pub fn new() -> Result<Self, DomainError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(TIMEOUT_SECS))
            .user_agent(concat!("openintel/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| DomainError::SourceFailure {
                name: "yahoo".into(),
                message: format!("client build failed: {e}"),
            })?;
        Ok(Self { client })
    }
}

#[async_trait]
impl MarketDataSource for YahooMarketSource {
    fn name(&self) -> &'static str {
        "yahoo"
    }

    async fn snapshot(&self, ticker: &Ticker) -> Result<MarketSnapshot, DomainError> {
        let url = format!("{BASE_URL}/{}?range=3mo&interval=1d", ticker.as_str());
        let fetched_at = Utc::now();

        let resp = self.client.get(&url).send().await.map_err(|e| {
            DomainError::SourceFailure {
                name: "yahoo".into(),
                message: format!("request failed: {e}"),
            }
        })?;
        let status = resp.status();
        let body = resp.text().await.map_err(|e| DomainError::SourceFailure {
            name: "yahoo".into(),
            message: format!("reading body failed (HTTP {status}): {e}"),
        })?;

        response::parse_snapshot(&body, ticker, fetched_at)
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p openintel --lib adapters::market::yahoo`
Expected: PASS (the `#[ignore]`d live test is skipped; count shows "1 passed; … 1 ignored" for the mod.rs tests, plus Task 1's parser tests).

- [ ] **Step 6: (Optional, network) live smoke test**

Run: `cargo test -p openintel --lib adapters::market::yahoo -- --ignored`
Expected: PASS if network is available. If no network, skip and note it — CI never runs this.

- [ ] **Step 7: Lint + format**

Run: `cargo clippy --all-targets -- -D warnings && cargo fmt`
Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml Cargo.lock src/adapters/market/yahoo/mod.rs
git commit -m "feat(market): Yahoo Finance HTTP adapter (reqwest/rustls)"
```

---

## Task 3: Inject the market source (DI composition root)

Pure refactor: thread `&dyn MarketDataSource` through the analysis path, construct the concrete `YahooMarketSource` at the two entry points, and inject `MockMarketSource` in every existing test. Behavior is unchanged; the existing suite is the safety net.

**Files:**
- Modify: `src/application/analyze.rs`
- Modify: `src/cli/run.rs`
- Modify: `src/main.rs`
- Modify: `src/mcp/tools.rs`
- Modify: `src/mcp/server.rs`
- Modify: `tests/analyze_flow.rs`

**Interfaces:**
- Consumes: `YahooMarketSource::new() -> Result<Self, DomainError>` and its `impl MarketDataSource` (Task 2); `MockMarketSource` (`src/adapters/market/mock_market.rs`, unit struct, `impl MarketDataSource`, `name()` → `"mock-market"`).
- Produces (new signatures):
  - `application::analyze(req: &AnalysisRequest, market_source: &dyn MarketDataSource) -> Result<SpeculationReport, DomainError>`
  - `cli::run::analyze(config: &AppConfig, market_source: &dyn MarketDataSource) -> Result<(SpeculationReport, String), DomainError>`
  - `mcp::tools::run_list_sources(market_source: &dyn MarketDataSource) -> SourcesOutput`
  - `mcp::tools::run_analyze(args, market_source: &dyn MarketDataSource) -> Result<AnalyzeOutput, DomainError>`
  - `mcp::tools::run_scan(args, market_source: &dyn MarketDataSource) -> ScanOutput`
  - `mcp::tools::run_compare(args, market_source: &dyn MarketDataSource) -> CompareOutput`
  - `mcp::server::OpenIntelServer::new(market: YahooMarketSource) -> Self` (field `market: YahooMarketSource`; `Default` removed)

- [ ] **Step 1: `application::analyze` — inject the source**

In `src/application/analyze.rs`:
1. Delete the line `use crate::adapters::market::mock_market::MockMarketSource;`.
2. Change the function signature and the market block:

```rust
pub async fn analyze(
    req: &AnalysisRequest,
    market_source: &dyn MarketDataSource,
) -> Result<SpeculationReport, DomainError> {
    let ticker = Ticker::parse(&req.ticker)?;
    let sources = build_sources(req);

    let fetches = sources.iter().map(|source| {
        let ticker = ticker.clone();
        async move { (source.kind(), source.fetch(&ticker, req.limit).await) }
    });
    let results = join_all(fetches).await;

    let mut posts: Vec<SocialPost> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    for (kind, result) in results {
        match result {
            Ok(mut fetched) => posts.append(&mut fetched),
            Err(e) => notes.push(format!("source {} failed: {e}", kind.as_str())),
        }
    }

    let market: Option<MarketSnapshot> = if req.market_enabled {
        match market_source.snapshot(&ticker).await {
            Ok(snapshot) => Some(snapshot),
            Err(e) => {
                notes.push(format!("market source failed: {e}"));
                None
            }
        }
    } else {
        None
    };

    if posts.is_empty() && market.is_none() {
        return Err(DomainError::NoData);
    }

    let analyzer = LexiconAnalyzer::new();
    let signals = analyzer.analyze(&posts).await?;

    let now = Utc::now();
    let mut report =
        SpeculationEngine::aggregate(&ticker, &posts, &signals, market.as_ref(), now, &req.engine)?;

    let engine_notes = std::mem::take(&mut report.fusion.notes);
    report.fusion.notes = notes.into_iter().chain(engine_notes).collect();

    Ok(report)
}
```

3. In the `#[cfg(test)] mod tests`, add `use crate::adapters::market::mock_market::MockMarketSource;` and update the two call sites:

```rust
    #[tokio::test]
    async fn analyzes_default_request_confirming_bullish() {
        use crate::domain::values::speculation::Alignment;
        let report = analyze(&req("AAPL", true), &MockMarketSource).await.unwrap();
        assert_eq!(report.social.total_mentions, 10);
        assert_eq!(report.fusion.alignment, Alignment::ConfirmingBullish);
        assert!(report.market.is_some());
    }

    #[tokio::test]
    async fn invalid_ticker_errors() {
        assert!(analyze(&req("$$$", true), &MockMarketSource).await.is_err());
    }
```

- [ ] **Step 2: `cli::run::analyze` — inject and forward**

In `src/cli/run.rs`:
1. Add to the top imports: `use crate::domain::ports::market_data_source::MarketDataSource;`.
2. Change the signature and the forward call:

```rust
pub async fn analyze(
    config: &AppConfig,
    market_source: &dyn MarketDataSource,
) -> Result<(SpeculationReport, String), DomainError> {
    let req = AnalysisRequest {
        ticker: config.ticker.clone(),
        enabled_sources: config.enabled_sources.clone(),
        market_enabled: config.market_enabled,
        limit: config.limit,
        engine: config.engine.clone(),
    };
    let report = application::analyze(&req, market_source).await?;
    let rendered = render(&report, config.format);
    Ok((report, rendered))
}
```

3. In `#[cfg(test)] mod tests`, add `use crate::adapters::market::mock_market::MockMarketSource;` and pass `&MockMarketSource` at all four call sites:

```rust
    #[tokio::test]
    async fn full_run_confirms_bullish_with_market() {
        let (report, rendered) = analyze(&config(false, OutputFormat::Json), &MockMarketSource)
            .await
            .unwrap();
        assert!(report.market.is_some());
        assert_eq!(report.fusion.alignment, Alignment::ConfirmingBullish);
        assert!(rendered.contains("Not financial advice"));
        assert!(rendered.contains("speculation_index"));
    }

    #[tokio::test]
    async fn no_market_run_is_quiet() {
        let (report, _) = analyze(&config(true, OutputFormat::Table), &MockMarketSource)
            .await
            .unwrap();
        assert!(report.market.is_none());
        assert_eq!(report.fusion.alignment, Alignment::Quiet);
    }

    #[tokio::test]
    async fn table_output_has_sections_and_disclaimer() {
        let (_, rendered) = analyze(&config(false, OutputFormat::Table), &MockMarketSource)
            .await
            .unwrap();
        assert!(rendered.contains("SOCIAL"));
        assert!(rendered.contains("MARKET"));
        assert!(rendered.contains("FUSION"));
        assert!(rendered.contains("Not financial advice"));
    }

    #[tokio::test]
    async fn invalid_ticker_errors() {
        let cfg = AppConfig::new("$$$".into(), false, false, false, false, 50, OutputFormat::Table);
        assert!(analyze(&cfg, &MockMarketSource).await.is_err());
    }
```

- [ ] **Step 3: `main.rs` — build Yahoo at the CLI root**

Replace `src/main.rs` with:

```rust
use std::process::ExitCode;

use clap::Parser;

use openintel::adapters::market::yahoo::YahooMarketSource;
use openintel::cli::args::{to_app_config, Cli, Command};
use openintel::cli::run::analyze;
use openintel::config::secrets::Credentials;

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    // Loaded for future keyed adapters; Yahoo (the current market source) needs no key.
    let _credentials = Credentials::from_env();

    match cli.command {
        Command::Analyze(args) => {
            let config = to_app_config(&args);
            let market = match YahooMarketSource::new() {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::FAILURE;
                }
            };
            match analyze(&config, &market).await {
                Ok((_report, rendered)) => {
                    println!("{rendered}");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Command::Mcp => match openintel::mcp::server::serve().await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("mcp server error: {e}");
                ExitCode::FAILURE
            }
        },
    }
}
```

- [ ] **Step 4: `mcp::tools` — thread the source through all four functions**

In `src/mcp/tools.rs` (the `use crate::domain::ports::market_data_source::MarketDataSource;` import already exists):

1. `run_list_sources` — report the injected source's name:

```rust
pub fn run_list_sources(market_source: &dyn MarketDataSource) -> SourcesOutput {
    SourcesOutput {
        social: SourceKind::ALL
            .iter()
            .map(|s| s.as_str().to_string())
            .collect(),
        market: vec![market_source.name().to_string()],
    }
}
```

2. `run_analyze` — accept and forward:

```rust
pub async fn run_analyze(
    args: AnalyzeArgs,
    market_source: &dyn MarketDataSource,
) -> Result<AnalyzeOutput, DomainError> {
    let req = request_from(
        args.ticker,
        args.enable_reddit,
        args.enable_x,
        args.enable_bluesky,
        args.no_market,
        args.limit,
    );
    let report = application::analyze(&req, market_source).await?;
    Ok(AnalyzeOutput {
        summary: summarize(&report),
        report,
        disclaimer: DISCLAIMER,
    })
}
```

3. `run_scan` — accept the source; each concurrent closure borrows it (a `&dyn` is `Copy`):

```rust
pub async fn run_scan(args: ScanArgs, market_source: &dyn MarketDataSource) -> ScanOutput {
    let ScanArgs {
        tickers,
        enable_reddit,
        enable_x,
        enable_bluesky,
        no_market,
        limit,
    } = args;
    let futures = tickers.into_iter().map(|t| async move {
        let req = request_from(t.clone(), enable_reddit, enable_x, enable_bluesky, no_market, limit);
        match application::analyze(&req, market_source).await {
            Ok(report) => ScanEntry {
                ticker: t,
                report: Some(report),
                error: None,
            },
            Err(e) => ScanEntry {
                ticker: t,
                report: None,
                error: Some(e.to_string()),
            },
        }
    });
    let entries = futures::future::join_all(futures).await;
    ScanOutput {
        entries,
        disclaimer: DISCLAIMER,
    }
}
```

4. `run_compare` — accept the source; forward inside the closure:

```rust
pub async fn run_compare(args: CompareArgs, market_source: &dyn MarketDataSource) -> CompareOutput {
    let CompareArgs {
        tickers,
        rank_by,
        enable_reddit,
        enable_x,
        enable_bluesky,
        no_market,
        limit,
    } = args;
    let futures = tickers.into_iter().map(|t| async move {
        let req = request_from(t.clone(), enable_reddit, enable_x, enable_bluesky, no_market, limit);
        (t, application::analyze(&req, market_source).await)
    });
    let results = futures::future::join_all(futures).await;

    let mut ranked: Vec<RankedEntry> = Vec::new();
    let mut errors: Vec<CompareError> = Vec::new();
    for (ticker, res) in results {
        match res {
            Ok(report) => {
                let metric = rank_metric(&report, rank_by);
                ranked.push(RankedEntry { ticker, rank_metric: metric, report });
            }
            Err(e) => errors.push(CompareError { ticker, error: e.to_string() }),
        }
    }
    sort_ranked(&mut ranked, rank_by);

    CompareOutput { rank_by, ranked, errors, disclaimer: DISCLAIMER }
}
```

5. In `#[cfg(test)] mod tests`, add `use crate::adapters::market::mock_market::MockMarketSource;` and update every call site to pass `&MockMarketSource`:
   - `run_list_sources()` → `run_list_sources(&MockMarketSource)` (assertion `vec!["mock-market"]` stays correct).
   - `run_analyze(args)` → `run_analyze(args, &MockMarketSource)` (both tests).
   - `run_scan(ScanArgs {..})` → `run_scan(ScanArgs {..}, &MockMarketSource)` (both tests).
   - `run_compare(CompareArgs {..})` → `run_compare(CompareArgs {..}, &MockMarketSource)`.
   - The `sort_ranked_orders_by_crowding_desc` test does not call any `run_*` function — leave it unchanged.

- [ ] **Step 5: `mcp::server` — own the source, build Yahoo in `serve()`**

In `src/mcp/server.rs`:
1. Add import: `use crate::adapters::market::yahoo::YahooMarketSource;`.
2. Change the struct + constructor, and delete the `Default` impl:

```rust
#[derive(Clone)]
pub struct OpenIntelServer {
    tool_router: ToolRouter<OpenIntelServer>,
    market: YahooMarketSource,
}

impl OpenIntelServer {
    pub fn new(market: YahooMarketSource) -> Self {
        Self {
            tool_router: Self::tool_router(),
            market,
        }
    }
}
```

(Delete the entire `impl Default for OpenIntelServer { … }` block.)

3. Pass `&self.market` in each tool method body:
   - `list_sources`: `serde_json::to_string_pretty(&tools::run_list_sources(&self.market))`
   - `analyze_ticker`: `tools::run_analyze(args, &self.market).await`
   - `scan_watchlist`: `tools::run_scan(args, &self.market).await`
   - `compare_tickers`: `tools::run_compare(args, &self.market).await`

4. Build the source in `serve()`:

```rust
pub async fn serve() -> Result<(), Box<dyn std::error::Error>> {
    let market = YahooMarketSource::new()?;
    let service = OpenIntelServer::new(market).serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
```

- [ ] **Step 6: `tests/analyze_flow.rs` — inject the mock**

In `tests/analyze_flow.rs`:
1. Add `use openintel::adapters::market::mock_market::MockMarketSource;`.
2. Pass `&MockMarketSource` at all three `analyze` call sites, e.g.:

```rust
#[tokio::test]
async fn end_to_end_all_sources_with_market() {
    let (report, json) = analyze(&cfg(false, false, false, false), &MockMarketSource)
        .await
        .unwrap();
    assert_eq!(report.social.total_mentions, 10);
    assert_eq!(report.fusion.alignment, Alignment::ConfirmingBullish);
    assert!(report.market.is_some());
    assert!(json.contains("\"alignment\": \"confirming_bullish\""));
    assert!(json.contains("Not financial advice"));
}

#[tokio::test]
async fn single_source_only() {
    let (report, _) = analyze(&cfg(true, false, false, false), &MockMarketSource)
        .await
        .unwrap();
    assert_eq!(report.social.total_mentions, 4);
}

#[tokio::test]
async fn social_only_when_market_disabled() {
    let (report, _) = analyze(&cfg(false, false, false, true), &MockMarketSource)
        .await
        .unwrap();
    assert!(report.market.is_none());
    assert_eq!(report.fusion.alignment, Alignment::Quiet);
}
```

- [ ] **Step 7: Run the whole suite (must stay green and hermetic)**

Run: `cargo test`
Expected: PASS — all prior tests plus Task 1/2 tests; the only `#[ignore]`d test is the Yahoo live test. No network access during the run.

- [ ] **Step 8: Lint + format + build**

Run: `cargo clippy --all-targets -- -D warnings && cargo fmt --check && cargo build`
Expected: clean; binary builds.

- [ ] **Step 9: Commit**

```bash
git add src/application/analyze.rs src/cli/run.rs src/main.rs src/mcp/tools.rs src/mcp/server.rs tests/analyze_flow.rs
git commit -m "refactor: inject MarketDataSource; wire Yahoo at composition roots"
```

---

## Task 4: Docs

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Update the Quickstart data-source note**

In `README.md`, replace the Quickstart caveat line:

```
> **Mock data today.** The numbers are illustrative until real data adapters land — a real market source is the next milestone.
```

with:

```
> **Market data is live (Yahoo Finance, keyless); social sources are still mock.** `analyze` fetches real price/volume over the network — offline runs degrade to a social-only report. Real social adapters are the next milestone.
```

- [ ] **Step 2: Update the Architecture + Extending notes**

In the `## Architecture` bullet list, change:

```
- `adapters/` — `LexiconAnalyzer` + mock data sources.
```

to:

```
- `adapters/` — `LexiconAnalyzer`, the `YahooMarketSource` (real, keyless), and mock social sources.
```

In the `## Extending` section, replace the "Add a market source" block:

```
**Add a market source** (e.g. Yahoo Finance):
1. New struct in `src/adapters/market/`, `impl MarketDataSource`.
2. Select it in `cli::run`.
```

with:

```
**Add a market source** (e.g. a keyed provider):
1. New struct in `src/adapters/market/`, `impl MarketDataSource`.
2. Construct it at the composition roots — `main.rs` (analyze branch) and `mcp::server::serve()` — and it flows in through the injected `&dyn MarketDataSource`. No engine or application change.
```

- [ ] **Step 3: Update the Risk section status line**

In the `⚠️ Risk & responsibility` section, change:

```
  product. OpenIntel itself is early software (mocked data sources today); the intelligence
  layer is meant to be iterated on.
```

to:

```
  product. OpenIntel itself is early software (live market data via Yahoo; social sources
  still mocked); the intelligence layer is meant to be iterated on.
```

- [ ] **Step 4: Commit**

```bash
git add README.md
git commit -m "docs: market data now live via Yahoo; DI extending note"
```

---

## Self-Review

**Spec coverage:**
- Keyless Yahoo v8 chart endpoint → Task 2 (URL) + Task 1 (parse). ✓
- Fetch/parse split, pure `parse_snapshot` → Tasks 1 & 2. ✓
- Field mapping table (last_price, previous_close, volume, avg_volume, realized_vol, None options, as_of) → Task 1 impl + tests. ✓
- `realized_vol` = √252·stdev(log returns), ≥20-returns gate → Task 1 `realized_vol` + tests. ✓
- DI signatures for application/cli/tools/server + composition roots (main, serve) → Task 3. ✓
- MCP server owns source, `Default` dropped → Task 3 Step 5. ✓
- `SourceFailure` error mapping, no unwrap on network data → Task 1 (`fail`) + Task 2 (mapped errors). ✓
- reqwest via `cargo add`, rustls, no OpenSSL → Task 2 Step 1. ✓
- Hermetic tests + one `#[ignore]` live test → Task 1/2 unit tests, Task 3 Step 7, Task 2 live test. ✓
- README updates → Task 4. ✓
- Non-goals (put_call/iv None, no retries/cache) → honored (fields None; single attempt). ✓

**Type consistency:** `market_source: &dyn MarketDataSource` used identically across `application::analyze`, `cli::run::analyze`, and the four `mcp::tools` functions. `YahooMarketSource::new() -> Result<Self, DomainError>` consumed in `main.rs` and `serve()`. `MockMarketSource` (unit struct) injected as `&MockMarketSource` in every test. `parse_snapshot(body, ticker, fetched_at)` produced in Task 1, consumed in Task 2. Consistent.

**Placeholder scan:** No TBD/TODO; every code step contains complete code.
