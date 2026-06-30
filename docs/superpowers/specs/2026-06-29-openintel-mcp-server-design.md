# OpenIntel — MCP Server Surface

**Status:** Reviewed — findings A–G applied (rmcp 2.x verified)
**Date:** 2026-06-29
**Type:** Feature — add an MCP server adapter alongside the existing CLI
**Builds on:** the hexagonal rewrite (`kloud/speculation-cli-rewrite`, PR #44)

---

## 1. Thesis

Expose openintel's speculation analysis as **MCP tools** so an AI agent (Claude Code on a subscription, ChatGPT, Codex, Cursor, Grok — whatever the user already runs) can consult it while trading through **Robinhood's official Agentic Trading MCP** (`https://agent.robinhood.com/mcp/trading`, launched 2026-05-27, equities beta).

openintel is the **intelligence layer**; the agent is the brain; Robinhood's MCP is execution. The composition lives in the agent:

```
agent (Claude Code / ChatGPT / Codex / Cursor / Grok)
  ├─ MCP → openintel                          (analysis — THIS spec)
  └─ MCP → agent.robinhood.com/mcp/trading    (execution, sandboxed agentic wallet)
```

**Hard boundary:** openintel's MCP is **analysis-only**. It never executes trades, never touches a broker, never holds brokerage credentials. The "API-key vs subscription" choice is the agent's concern, upstream of openintel — openintel is agent-agnostic.

---

## 2. Scope

### This spec
- A new `application/` use-case layer (small refactor) so CLI and MCP share one orchestration path.
- An `mcp/` driving adapter built on `rmcp` 2.0 over **stdio**.
- Four read-only tools: `analyze_ticker`, `scan_watchlist`, `list_sources`, `compare_tickers`.
- A new `openintel mcp` subcommand to launch the stdio server.
- README section: wiring + safety guidance.

### Non-goals / out of scope
- No trade execution, broker integration, or brokerage credentials — ever, in this surface.
- No HTTP transport in v1 (stdio only; HTTP is a clean later seam).
- No persistence, watchlist storage, or alerting.
- The domain core (`SpeculationEngine`, ports, value objects) is **unchanged**.

---

## 3. Architecture

The MCP server is a second **inbound/driving adapter** beside the CLI, over the same pure domain.

```
src/
  application/
    mod.rs            # re-exports + the shared DISCLAIMER const (reused by cli + mcp)
    request.rs        # NEW: AnalysisRequest — presentation-free use-case input
    analyze.rs        # NEW: analyze(&AnalysisRequest) -> Result<SpeculationReport, DomainError>
                      #      orchestration only (build sources → concurrent fetch → analyzer
                      #      → engine.aggregate, clock injected here); NO rendering
  cli/
    run.rs            # AppConfig -> AnalysisRequest -> application::analyze -> render table|json
    args.rs           # clap; Command gains an `Mcp` variant
  mcp/
    mod.rs
    server.rs         # rmcp server: thin #[tool] wrappers (Json<_>/ErrorData) + stdio startup
    tools.rs          # tool LOGIC fns (typed *Output structs) + input types + ranking
  domain/
    values/source_kind.rs   # + `SourceKind::ALL` — single source-of-truth list for sources
  adapters/…          # UNCHANGED (mock sources, lexicon, mock market)
  config/…            # UNCHANGED (AppConfig, Credentials)
```

**Use-case extraction (the refactor):** today `cli::run::analyze` fuses orchestration with rendering. Split it so both surfaces share a **presentation-free** input + use case:

```rust
// src/application/request.rs
pub struct AnalysisRequest {
    pub ticker: String,
    pub enabled_sources: Vec<SourceKind>,
    pub market_enabled: bool,
    pub limit: usize,
    pub engine: EngineConfig,
}

// src/application/analyze.rs
pub async fn analyze(req: &AnalysisRequest) -> Result<SpeculationReport, DomainError>;
// builds enabled SocialDataSources + market source, concurrent fetch (per-source failure
// non-fatal → note), LexiconAnalyzer, reads `Utc::now()` HERE, SpeculationEngine::aggregate,
// merges orchestration notes. Returns NoData error when no posts and no market.
```

`AnalysisRequest` carries **only analysis params** — no `format`, no presentation. The CLI maps `AppConfig → AnalysisRequest` and keeps `format` for *its* renderer; MCP builds `AnalysisRequest` directly from tool args. **Neither fabricates a placeholder format.** `cli::run::analyze` becomes a thin wrapper: `let report = application::analyze(&req).await?; let rendered = render(&report, format); Ok((report, rendered))` — all 46 existing tests stay green (the CLI just delegates).

A single `DISCLAIMER` const lives in `application` and is reused by the CLI renderer and the MCP outputs (no duplication).

---

## 4. The four tools

All inputs derive `serde::Deserialize` + `schemars::JsonSchema`; all outputs derive `serde::Serialize`. Optional fields default to the existing engine/CLI defaults (no source flags → all three enabled; market on; `limit` 50).

### `analyze_ticker`
```
in:  { ticker: String, enable_reddit?: bool, enable_x?: bool, enable_bluesky?: bool,
       no_market?: bool, limit?: usize }
out: { report: SpeculationReport, disclaimer: String }
```
Builds an `AnalysisRequest` from the args, calls `application::analyze`, returns the serialized report. Invalid ticker → the `#[tool]` wrapper returns `Err(rmcp::ErrorData)` carrying the `DomainError` message (a tool error).

### `scan_watchlist`
```
in:  { tickers: [String], <same opts> }
out: { entries: [ { ticker: String, report?: SpeculationReport, error?: String } ],
       disclaimer: String }
```
Runs `application::analyze` per ticker **concurrently** (`futures::join_all`). A per-ticker failure (invalid symbol, `NoData`) becomes an `error` entry — it does **not** fail the batch.

### `list_sources`
```
in:  {}
out: { social: [String], market: [String] }
```
Derived from **`SourceKind::ALL`** (the single source-of-truth list the CLI source-builder also iterates) via `SourceKind::as_str`, plus the market adapter's `name()`. v1 yields `social = ["reddit","x","bluesky"]`, `market = ["mock-market"]` — and stays correct automatically when a source is added. (No disclaimer — pure metadata.)

### `compare_tickers`
```
in:  { tickers: [String], rank_by?: RankBy, <same opts> }
out: { rank_by: RankBy,
       ranked: [ { ticker: String, rank_metric: f64, report: SpeculationReport } ],  // desc
       errors: [ { ticker: String, error: String } ],
       disclaimer: String }
```
`RankBy ∈ { crowding (default), speculation_index, net_sentiment, divergence }` (serde/schemars `snake_case`). Ranking, descending:
- `crowding` → `report.fusion.crowding`
- `speculation_index` → `report.social.speculation_index.value()`
- `net_sentiment` → `report.social.net_sentiment.value()` (most bullish first)
- `divergence` → diverging-first then crowding: sort key `(alignment == Diverging, crowding)` desc; `rank_metric` reported = `crowding`.

Failed tickers go to `errors`, excluded from `ranked`. Sorting uses `partial_cmp` with an `Equal` fallback — rank metrics are finite by construction (the engine clamps and guards div-by-zero), so no NaN, but `partial_cmp` is still the correct form.

**Batch behavior (`scan_watchlist`, `compare_tickers`):** an empty `tickers` list returns an empty result (not an error); duplicate tickers are processed as-is. No max-list cap in v1 — unbounded fan-out is harmless against mocks; a cap belongs with the real-adapter work.

---

## 5. Output format & disclaimer

Each tool handler returns **`Json<Output>`** — rmcp's wrapper that emits the serialized struct as MCP **`structuredContent`** (with an auto-generated `outputSchema`). Outputs are the typed structs in §4 (`SpeculationReport` already derives `Serialize`). Alongside the structured data, handlers include a one-line **text summary** for clients that read text content, formatted like:

```
AAPL — ConfirmingBullish · crowding 50% · 10 mentions (Medium)
```

Every analysis-bearing result (`analyze_ticker`, `scan_watchlist`, `compare_tickers`) carries the shared **`DISCLAIMER`** const in a `disclaimer` field (*"Not financial advice. Sentiment is noisy and easily manipulated; verify before acting."*); `list_sources` omits it (pure metadata).

Rationale: agents reason over fields, not ASCII tables — so structured JSON here, even though the CLI keeps its human table.

---

## 6. Runtime & dependencies

- **Transport:** stdio. New subcommand `openintel mcp` constructs the server and `serve`s it over stdin/stdout (blocking). The existing `analyze` subcommand is unchanged.
- **SDK (rmcp 2.x — verified against the 2.x docs):** `#[tool_router]` on the server impl, `#[tool]` on each async handler (auto-generates the input JSON schema via `schemars` + routing), `#[tool_handler]` for `ServerHandler`, `Parameters<T>` inputs, and `Json<Output>` returns (→ `structuredContent`). Handlers return `Result<Json<Output>, rmcp::ErrorData>`. Startup: `serve_server(stdio())` (`rmcp::transport::io::stdio`).
- **Cargo additions:** `rmcp = { version = "2", features = ["transport-io"] }` (`server` + `macros` are default features), `schemars = "1"`. `tokio`/`serde`/`serde_json`/`futures` already present. Still no `reqwest`/DB.
- **stdout discipline (critical):** stdout is the MCP protocol channel. **Nothing** in the MCP path may `println!`. Diagnostics use `eprintln!` (stderr). No `tracing`/`tracing-subscriber` dependency in v1 — silence on stdout is enforced by discipline, covered by review.

Wiring (documented in README):
```
claude mcp add openintel -- openintel mcp
```

---

## 7. Testing

- **Unchanged:** all 46 existing tests stay green — `cli::run::analyze` now delegates to `application::analyze`.
- **`application::analyze`:** returns a populated `ConfirmingBullish` report for the default config (the behavior previously asserted in `cli::run`); `--no-market` → `market: None`, `Quiet`; invalid ticker → `Err`.
- **Tool logic is split from the rmcp wrapper:** each tool is a plain typed logic fn (`run_analyze(&self, args) -> Result<AnalyzeOutput, DomainError>`, etc.) plus a thin `#[tool]` wrapper that packages `Json<_>` / `ErrorData`. Tests call the **logic fns** and assert on the typed `*Output` structs — no MCP types in the assertions, no subprocess:
  - `run_analyze` → report present, `ConfirmingBullish`, disclaimer set.
  - `run_scan(["AAPL", "$$$"])` → one `report` entry + one `error` entry (batch survives); `run_scan([])` → empty.
  - `run_compare(["AAPL","MSFT"], crowding)` → `ranked` ordered desc; bad tickers in `errors`.
  - `run_list_sources` → `social` contains reddit/x/bluesky, `market` contains `mock-market`.

---

## 8. Risks & boundary

- **Analysis-only, enforced by omission.** No broker code, no execution path, no credentials. The agent — gated by Robinhood's own approval mode + funded-wallet cap — owns execution.
- **Agent composition is inherent to MCP** (agents connect to many servers); Robinhood doesn't document openintel-alongside-Robinhood specifically, but it's how MCP works — low risk.
- **Responsible-use guidance** in README: keep Robinhood's *approval-required* mode on, fund a deliberately small agentic wallet (blast-radius cap), treat output as a screen, not advice.
- **Beta reality:** Robinhood agentic execution is equities-only today; openintel's *analysis* spans the same tickers regardless, but options/crypto execution isn't live upstream yet.

---

## 9. Decisions log

1. MCP server is a new **inbound adapter** beside the CLI; domain core untouched.
2. Extract a thin **`application::analyze`** use case taking a presentation-free **`AnalysisRequest`** (no `format`); CLI and MCP both build it.
3. **Richer tool set:** `analyze_ticker`, `scan_watchlist`, `list_sources`, `compare_tickers`.
4. **stdio** transport (local subprocess); HTTP deferred.
5. **`Json<Output>` → structuredContent** (+ a one-line text summary); disclaimer on every analysis result.
6. `rmcp` 2.x (`transport-io` feature; `serve_server(stdio())`; handlers `-> Result<Json<_>, ErrorData>`) + `schemars` 1; stdout reserved for MCP, diagnostics to stderr; no `tracing` dep.
7. **Hard boundary:** analysis-only — never trades. Robinhood's MCP executes; the agent composes.
8. **Tool logic split from the rmcp wrapper** — typed `*Output` logic fns (tested directly) + thin `#[tool]` wrappers.
9. **`list_sources` derived from `SourceKind::ALL`** (one source-of-truth list, shared with the CLI builder) — no hardcoded drift.
10. **One shared `DISCLAIMER` const** in `application`, reused by CLI + MCP; `compare` sorts via `partial_cmp`; empty list → empty result.
