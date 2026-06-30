# OpenIntel MCP Server — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an analysis-only MCP server surface (over stdio) beside the existing CLI, so an AI agent can consult openintel's speculation analysis alongside Robinhood's official trading MCP.

**Architecture:** Extract the orchestration into a presentation-free `application::analyze` use case; add an `mcp` inbound adapter (rmcp 2.x) that exposes four read-only tools over the same domain core; wire an `openintel mcp` subcommand. The pure domain (`SpeculationEngine`, ports) is untouched.

**Tech Stack:** Rust 2021, Tokio, `rmcp` 2.x (official MCP SDK), `schemars` 1, serde, futures.

**Spec:** `docs/superpowers/specs/2026-06-29-openintel-mcp-server-design.md`
**Reference (rmcp 2.x server pattern):** https://github.com/modelcontextprotocol/rust-sdk — `examples/servers/src/common/counter.rs`

## Global Constraints

- **Stacks on the rewrite.** This work is on branch `kloud/mcp-server` (already contains the full hexagonal CLI: `domain/`, `adapters/`, `config/`, `cli/`, 46 passing tests). Do not modify `domain/` except to add `SourceKind::ALL`.
- **Dependencies:** add `rmcp = { version = "2", features = ["transport-io"] }` (its `server` + `macros` features are on by default) and `schemars = "1"`. No `reqwest`, DB, `tracing`, or `anyhow`.
- **rmcp 2.x surface (verified):** tool handlers live in a `#[tool_router] impl`, each `#[tool(description="…")]`; inputs via `Parameters<T>`; return `Result<Json<Output>, rmcp::ErrorData>` (the `Json<T>` wrapper emits `structuredContent`); the server struct holds `tool_router: ToolRouter<Self>` set in `new()` via `Self::tool_router()`; `#[tool_handler] impl ServerHandler` provides `get_info`; start with `OpenIntelServer::new().serve(stdio()).await` (`rmcp::ServiceExt`, `rmcp::transport::io::stdio`). If any exact symbol differs in the installed rmcp 2.x, consult the counter example linked above.
- **Analysis-only — the MCP surface NEVER executes trades, touches a broker, or holds credentials.** Robinhood's MCP does execution; the agent composes the two.
- **stdout discipline:** stdout is the MCP protocol channel. No `println!` anywhere in the `mcp`/`application` path. Diagnostics use `eprintln!` (stderr).
- **One shared disclaimer:** a single `application::DISCLAIMER` const, reused by the CLI renderer and the MCP outputs.
- **Every task ends green:** `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` must pass before the commit step.
- **Commit trailer:** end each commit body with `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`.

---

## File Structure

```
Cargo.toml                              # + rmcp, schemars                      (Task 2)
src/lib.rs                              # + pub mod application; pub mod mcp;    (Tasks 1,2)
src/domain/values/source_kind.rs        # + SourceKind::ALL                      (Task 1)
src/application/
  mod.rs                                # DISCLAIMER const + re-exports          (Task 1)
  request.rs                            # AnalysisRequest                        (Task 1)
  analyze.rs                            # analyze(&AnalysisRequest) -> Report     (Task 1)
src/cli/run.rs                          # thin: AppConfig->AnalysisRequest        (Task 1)
src/cli/args.rs                         # Command gains `Mcp`                    (Task 2)
src/main.rs                             # handle Command::Mcp                    (Task 2)
src/config/settings.rs                  # AppConfig::new uses SourceKind::ALL    (Task 1)
src/mcp/
  mod.rs                                # pub mod server; pub mod tools;         (Task 2)
  server.rs                             # rmcp server + #[tool] wrappers + serve (Tasks 2-5)
  tools.rs                              # tool logic fns + I/O types + ranking   (Tasks 2-5)
README.md                              # "Use with an AI agent (MCP)" section   (Task 6)
```

---

### Task 1: Application use-case layer (the refactor)

Extract orchestration from `cli/run.rs` into `application::analyze`, add `AnalysisRequest`, hoist the disclaimer, and add `SourceKind::ALL`. Behavior is unchanged — the 46 existing tests are the safety net.

**Files:**
- Modify: `src/domain/values/source_kind.rs`, `src/config/settings.rs`, `src/cli/run.rs`, `src/lib.rs`
- Create: `src/application/mod.rs`, `src/application/request.rs`, `src/application/analyze.rs`

**Interfaces:**
- Produces:
  - `SourceKind::ALL: [SourceKind; 3]`
  - `application::DISCLAIMER: &'static str`
  - `application::request::AnalysisRequest { ticker: String, enabled_sources: Vec<SourceKind>, market_enabled: bool, limit: usize, engine: EngineConfig }`
  - `application::analyze(req: &AnalysisRequest) -> Result<SpeculationReport, DomainError>` (async)
- Consumes: existing adapters, `SpeculationEngine`, ports, `AppConfig`.

- [ ] **Step 1: Add `SourceKind::ALL` + test** — in `src/domain/values/source_kind.rs`, add the const inside the existing `impl SourceKind` (above `as_str`), and a test.

```rust
impl SourceKind {
    /// The full set of social sources, in canonical order — the single source of
    /// truth used by the CLI source-builder, AppConfig defaults, and `list_sources`.
    pub const ALL: [SourceKind; 3] = [SourceKind::Reddit, SourceKind::X, SourceKind::Bluesky];
```

Add to the file's `#[cfg(test)] mod tests`:
```rust
    #[test]
    fn all_lists_every_variant_in_order() {
        assert_eq!(
            SourceKind::ALL,
            [SourceKind::Reddit, SourceKind::X, SourceKind::Bluesky]
        );
    }
```

- [ ] **Step 2: Use `SourceKind::ALL` in `AppConfig::new`** — in `src/config/settings.rs`, replace the hardcoded all-sources default. Find:
```rust
        if enabled_sources.is_empty() {
            enabled_sources = vec![SourceKind::Reddit, SourceKind::X, SourceKind::Bluesky];
        }
```
Replace with:
```rust
        if enabled_sources.is_empty() {
            enabled_sources = SourceKind::ALL.to_vec();
        }
```

- [ ] **Step 3: Create `src/application/mod.rs`**

```rust
pub mod analyze;
pub mod request;

pub use analyze::analyze;
pub use request::AnalysisRequest;

/// Appended to every analysis-bearing output (CLI renders it; MCP returns it in a
/// `disclaimer` field). Single source of truth — do not duplicate this string.
pub const DISCLAIMER: &str = "Not financial advice. OpenIntel is a research/screening tool; \
markets are risky and social data is easily manipulated. Do your own diligence.";
```

- [ ] **Step 4: Create `src/application/request.rs`**

```rust
use crate::domain::engine::config::EngineConfig;
use crate::domain::values::source_kind::SourceKind;

/// Presentation-free input to the analysis use case. Carries only analysis
/// parameters — no output format or rendering concerns (those belong to the
/// driving adapter: CLI or MCP).
#[derive(Debug, Clone)]
pub struct AnalysisRequest {
    pub ticker: String,
    pub enabled_sources: Vec<SourceKind>,
    pub market_enabled: bool,
    pub limit: usize,
    pub engine: EngineConfig,
}
```

- [ ] **Step 5: Create `src/application/analyze.rs`** (orchestration moved out of `cli/run.rs`, now over `AnalysisRequest`)

```rust
use chrono::Utc;
use futures::future::join_all;

use crate::adapters::analyzer::lexicon::LexiconAnalyzer;
use crate::adapters::market::mock_market::MockMarketSource;
use crate::adapters::sources::mock_bluesky::MockBlueskySource;
use crate::adapters::sources::mock_reddit::MockRedditSource;
use crate::adapters::sources::mock_x::MockXSource;
use crate::application::request::AnalysisRequest;
use crate::domain::engine::speculation_engine::SpeculationEngine;
use crate::domain::entities::market_snapshot::MarketSnapshot;
use crate::domain::entities::social_post::SocialPost;
use crate::domain::entities::speculation_report::SpeculationReport;
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::ports::market_data_source::MarketDataSource;
use crate::domain::ports::post_analyzer::PostAnalyzer;
use crate::domain::ports::social_data_source::SocialDataSource;
use crate::domain::values::source_kind::SourceKind;

fn build_sources(req: &AnalysisRequest) -> Vec<Box<dyn SocialDataSource>> {
    req.enabled_sources
        .iter()
        .map(|kind| -> Box<dyn SocialDataSource> {
            match kind {
                SourceKind::Reddit => Box::new(MockRedditSource),
                SourceKind::X => Box::new(MockXSource),
                SourceKind::Bluesky => Box::new(MockBlueskySource),
            }
        })
        .collect()
}

pub async fn analyze(req: &AnalysisRequest) -> Result<SpeculationReport, DomainError> {
    let ticker = Ticker::parse(&req.ticker)?;
    let sources = build_sources(req);

    let fetches = sources
        .iter()
        .map(|source| async move { (source.kind(), source.fetch(&ticker, req.limit).await) });
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
        match MockMarketSource.snapshot(&ticker).await {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn req(ticker: &str, market: bool) -> AnalysisRequest {
        AnalysisRequest {
            ticker: ticker.into(),
            enabled_sources: SourceKind::ALL.to_vec(),
            market_enabled: market,
            limit: 50,
            engine: crate::domain::engine::config::EngineConfig::default(),
        }
    }

    #[tokio::test]
    async fn analyzes_default_request_confirming_bullish() {
        use crate::domain::values::speculation::Alignment;
        let report = analyze(&req("AAPL", true)).await.unwrap();
        assert_eq!(report.social.total_mentions, 10);
        assert_eq!(report.fusion.alignment, Alignment::ConfirmingBullish);
        assert!(report.market.is_some());
    }

    #[tokio::test]
    async fn invalid_ticker_errors() {
        assert!(analyze(&req("$$$", true)).await.is_err());
    }
}
```

- [ ] **Step 6: Register the module** — in `src/lib.rs`, add `pub mod application;` (keep alphabetical):
```rust
pub mod adapters;
pub mod application;
pub mod cli;
pub mod config;
pub mod domain;
```

- [ ] **Step 7: Rewrite `src/cli/run.rs` as a thin wrapper** — replace the WHOLE file with the following. The orchestration is gone (now in `application`); rendering stays; `DISCLAIMER` comes from `application`. **Keep the existing `#[cfg(test)] mod tests` block at the end unchanged** — append it verbatim from the current file (the four tests still call `analyze(&config)`).

```rust
use crate::application::{self, request::AnalysisRequest, DISCLAIMER};
use crate::config::settings::{AppConfig, OutputFormat};
use crate::domain::entities::speculation_report::SpeculationReport;
use crate::domain::error::DomainError;

pub async fn analyze(config: &AppConfig) -> Result<(SpeculationReport, String), DomainError> {
    let req = AnalysisRequest {
        ticker: config.ticker.clone(),
        enabled_sources: config.enabled_sources.clone(),
        market_enabled: config.market_enabled,
        limit: config.limit,
        engine: config.engine.clone(),
    };
    let report = application::analyze(&req).await?;
    let rendered = render(&report, config.format);
    Ok((report, rendered))
}

fn render(report: &SpeculationReport, format: OutputFormat) -> String {
    match format {
        OutputFormat::Json => render_json(report),
        OutputFormat::Table => render_table(report),
    }
}

fn render_json(report: &SpeculationReport) -> String {
    #[derive(serde::Serialize)]
    struct Envelope<'a> {
        #[serde(flatten)]
        report: &'a SpeculationReport,
        disclaimer: &'static str,
    }
    serde_json::to_string_pretty(&Envelope { report, disclaimer: DISCLAIMER })
        .unwrap_or_else(|e| format!("{{\"error\":\"serialization failed: {e}\"}}"))
}

fn render_table(report: &SpeculationReport) -> String {
    use std::fmt::Write;

    let mut out = String::new();
    let s = &report.social;
    let _ = writeln!(out, "=== OpenIntel — {} ===", report.ticker.as_str());
    let _ = writeln!(out, "generated: {}", report.generated_at.to_rfc3339());
    let _ = writeln!(out, "confidence (social sample): {:?}", report.social_confidence);
    let _ = writeln!(out, "\nSOCIAL");
    let _ = writeln!(
        out,
        "  mentions: {} (bull {} / bear {} / neutral {})",
        s.total_mentions, s.bullish, s.bearish, s.neutral
    );
    let _ = writeln!(out, "  net sentiment: {:+.2}", s.net_sentiment.value());
    let _ = writeln!(out, "  speculation index: {:.0}%", s.speculation_index.value() * 100.0);
    match s.bull_bear_ratio {
        Some(r) => {
            let _ = writeln!(out, "  bull/bear ratio: {r:.2}");
        }
        None => {
            let _ = writeln!(out, "  bull/bear ratio: n/a (no bearish posts)");
        }
    }

    match &report.market {
        Some(m) => {
            let _ = writeln!(out, "\nMARKET");
            let _ = writeln!(
                out,
                "  last: {:.2}  change: {:+.2}%  rvol: {:.2}x",
                m.last_price, m.pct_change, m.rvol
            );
        }
        None => {
            let _ = writeln!(out, "\nMARKET\n  (disabled)");
        }
    }

    let _ = writeln!(out, "\nFUSION");
    let _ = writeln!(out, "  alignment: {:?}", report.fusion.alignment);
    let _ = writeln!(out, "  crowding: {:.0}%", report.fusion.crowding * 100.0);
    for note in &report.fusion.notes {
        let _ = writeln!(out, "  note: {note}");
    }

    let _ = writeln!(out, "\n{DISCLAIMER}");
    out
}

// >>> KEEP the existing `#[cfg(test)] mod tests { ... }` block from the current
//     cli/run.rs here, unchanged. Its four tests still call `analyze(&config)`.
```

- [ ] **Step 8: Run the full suite + lint**

Run: `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: all pass (46 prior + the 2 new `application::analyze` tests + 1 new `SourceKind::ALL` test). No unused-import warnings (the orchestration imports were removed from `cli/run.rs`).

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "refactor: extract application::analyze use case + SourceKind::ALL

Presentation-free AnalysisRequest shared by CLI (and the upcoming MCP surface);
single DISCLAIMER const. Behavior unchanged — existing tests green.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 2: rmcp scaffolding + `list_sources` + `openintel mcp` subcommand

Stand up the rmcp 2.x stdio server with the first (simplest) real tool. This task also verifies the rmcp 2.x API end-to-end (deps resolve, macros compile, `Json<_>` return works, stdio starts).

**Files:**
- Modify: `Cargo.toml`, `src/lib.rs`, `src/cli/args.rs`, `src/main.rs`
- Create: `src/mcp/mod.rs`, `src/mcp/tools.rs`, `src/mcp/server.rs`

**Interfaces:**
- Consumes: `SourceKind::ALL`, `MockMarketSource`, `MarketDataSource`.
- Produces:
  - `mcp::tools::SourcesOutput { social: Vec<String>, market: Vec<String> }`, `mcp::tools::run_list_sources() -> SourcesOutput`
  - `mcp::server::OpenIntelServer`, `mcp::server::serve() -> Result<(), Box<dyn std::error::Error>>`
  - `cli::args::Command::Mcp`

- [ ] **Step 1: Add dependencies** — in `Cargo.toml` `[dependencies]`, add:
```toml
rmcp = { version = "2", features = ["transport-io"] }
schemars = "1"
```
> **schemars/rmcp alignment:** rmcp 2.x re-exports schemars (it derives the tool *input* schemas). `schemars = "1"` should match rmcp 2.x's schemars-1.x line. If the build complains that the derived `JsonSchema` doesn't satisfy rmcp's `Parameters<T>` bound (a version skew), drop the direct dep and derive via rmcp's re-export instead: `use rmcp::schemars;` + `#[derive(schemars::JsonSchema)]` with `#[schemars(crate = "rmcp::schemars")]` on each input type. Resolve this here in Task 2, before building the real tools.

- [ ] **Step 2: Write the failing test** — create `src/mcp/tools.rs` with the test first:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_sources_reports_all_adapters() {
        let out = run_list_sources();
        assert_eq!(out.social, vec!["reddit", "x", "bluesky"]);
        assert_eq!(out.market, vec!["mock-market"]);
    }
}
```

- [ ] **Step 3: Run it to verify it fails**

Run: `cargo test mcp::tools 2>&1 | tail -5`
Expected: FAIL — `run_list_sources`/module not found (and `mcp` not yet declared).

- [ ] **Step 4: Implement `run_list_sources`** — prepend to `src/mcp/tools.rs`:
```rust
use serde::Serialize;

use crate::domain::ports::market_data_source::MarketDataSource;
use crate::domain::values::source_kind::SourceKind;

// NOTE: Serialize-ONLY (no JsonSchema). This is deliberate — it makes `list_sources`
// returning `Json<SourcesOutput>` the spike that proves `Json<T>` works with a
// Serialize-only payload. The report-bearing outputs (Tasks 3-5) nest the
// Serialize-only `SpeculationReport`, so they cannot derive JsonSchema. If this
// compiles, `Json<T>` needs only Serialize and those tools are fine as written.
#[derive(Debug, Serialize)]
pub struct SourcesOutput {
    pub social: Vec<String>,
    pub market: Vec<String>,
}

/// Derived from `SourceKind::ALL` (one source of truth) + the market adapter's name.
pub fn run_list_sources() -> SourcesOutput {
    SourcesOutput {
        social: SourceKind::ALL.iter().map(|s| s.as_str().to_string()).collect(),
        market: vec![crate::adapters::market::mock_market::MockMarketSource.name().to_string()],
    }
}
```

- [ ] **Step 5: Write the rmcp server** — create `src/mcp/server.rs`:
```rust
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Json;
use rmcp::model::{Implementation, ServerCapabilities, ServerInfo};
use rmcp::transport::io::stdio;
use rmcp::{tool, tool_handler, tool_router, ErrorData, ServerHandler, ServiceExt};

use crate::mcp::tools;

#[derive(Clone)]
pub struct OpenIntelServer {
    tool_router: ToolRouter<OpenIntelServer>,
}

impl OpenIntelServer {
    pub fn new() -> Self {
        Self { tool_router: Self::tool_router() }
    }
}

impl Default for OpenIntelServer {
    fn default() -> Self {
        Self::new()
    }
}

#[tool_router]
impl OpenIntelServer {
    #[tool(description = "List the social and market data sources OpenIntel can analyze. Read-only metadata.")]
    async fn list_sources(&self) -> Result<Json<tools::SourcesOutput>, ErrorData> {
        Ok(Json(tools::run_list_sources()))
    }
}

#[tool_handler]
impl ServerHandler for OpenIntelServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::from_build_env())
            .with_instructions(
                "OpenIntel — fuses social sentiment with market action into a speculation \
                 report (crowding, divergence, sentiment). READ-ONLY: it never places trades.",
            )
    }
}

/// Run the MCP server over stdio (blocks until the client disconnects).
pub async fn serve() -> Result<(), Box<dyn std::error::Error>> {
    let service = OpenIntelServer::new().serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
```
> If the installed rmcp 2.x differs on `ServerInfo::new` / `with_*` / `ServiceExt::serve` / `ToolRouter` import paths, mirror the counter example at the reference URL (it is the same rmcp 2.x line) and adjust — keep the tool/handler shape identical.

- [ ] **Step 6: Create `src/mcp/mod.rs`**
```rust
pub mod server;
pub mod tools;
```

- [ ] **Step 7: Register `mcp` + add the `Mcp` subcommand**

`src/lib.rs` — add `pub mod mcp;` (alphabetical, after `domain`):
```rust
pub mod adapters;
pub mod application;
pub mod cli;
pub mod config;
pub mod domain;
pub mod mcp;
```

`src/cli/args.rs` — add a variant to `Command` (leave `Analyze` and `to_app_config` unchanged):
```rust
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Analyze a ticker across social + market sources
    Analyze(AnalyzeArgs),

    /// Run as an MCP server over stdio (for AI agents).
    Mcp,
}
```

`src/main.rs` — add the `Mcp` arm to the `match cli.command`:
```rust
        Command::Mcp => match openintel::mcp::server::serve().await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("mcp server error: {e}");
                ExitCode::FAILURE
            }
        },
```

- [ ] **Step 8: Verify build, tests, and that the server starts**

Run: `cargo build && cargo test mcp::tools && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: builds clean (rmcp/schemars resolve, macros compile), `list_sources_reports_all_adapters` passes, lint clean.

> **If the build fails on the `Json<SourcesOutput>` return** with a missing `JsonSchema`/output-schema bound, then `Json<T>` requires `T: JsonSchema` — which the report-bearing outputs can't satisfy without deriving JsonSchema across the whole domain. In that case, switch every tool wrapper (this one + Tasks 3-5) to return `CallToolResult` instead, packing the JSON as a text block (needs only `Serialize`):
> ```rust
> async fn list_sources(&self) -> Result<rmcp::model::CallToolResult, ErrorData> {
>     let json = serde_json::to_string_pretty(&tools::run_list_sources())
>         .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
>     Ok(rmcp::model::CallToolResult::success(vec![rmcp::model::ContentBlock::text(json)]))
> }
> ```
> Decide here in Task 2 and apply the same return shape consistently in Tasks 3-5.

Run (smoke — the server should start and wait on stdin, then exit on EOF):
`echo "" | cargo run --quiet -- mcp; echo "exit=$?"`
Expected: no stdout noise besides MCP framing (likely nothing on empty input), process exits (EOF closes stdin); `exit=0`.

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "feat(mcp): rmcp stdio server scaffold + list_sources tool + mcp subcommand

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 3: `analyze_ticker` tool

**Files:**
- Modify: `src/mcp/tools.rs`, `src/mcp/server.rs`

**Interfaces:**
- Consumes: `application::analyze`, `application::DISCLAIMER`, `AnalysisRequest`, `EngineConfig`, `SourceKind::ALL`, `SpeculationReport`.
- Produces: `tools::AnalyzeArgs`, `tools::AnalyzeOutput`, `tools::request_from(...)`, `tools::summarize(&report)`, `tools::run_analyze(args) -> Result<AnalyzeOutput, DomainError>`.

- [ ] **Step 1: Write the failing test** — append to `src/mcp/tools.rs` `mod tests`:
```rust
    #[tokio::test]
    async fn run_analyze_returns_confirming_bullish_report() {
        let out = run_analyze(AnalyzeArgs {
            ticker: "AAPL".into(),
            enable_reddit: None,
            enable_x: None,
            enable_bluesky: None,
            no_market: None,
            limit: None,
        })
        .await
        .unwrap();
        assert!(out.summary.contains("ConfirmingBullish"));
        assert_eq!(out.report.social.total_mentions, 10);
        assert!(out.disclaimer.contains("Not financial advice"));
    }

    #[tokio::test]
    async fn run_analyze_rejects_bad_ticker() {
        let args = AnalyzeArgs {
            ticker: "$$$".into(),
            enable_reddit: None, enable_x: None, enable_bluesky: None,
            no_market: None, limit: None,
        };
        assert!(run_analyze(args).await.is_err());
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test mcp::tools::tests::run_analyze`
Expected: FAIL — `run_analyze`/`AnalyzeArgs` not found.

- [ ] **Step 3: Implement the tool logic** — add to `src/mcp/tools.rs` (above the test module). Add the imports at the top of the file alongside the existing ones:
```rust
use schemars::JsonSchema;
use serde::Deserialize;

use crate::application::{self, request::AnalysisRequest, DISCLAIMER};
use crate::domain::engine::config::EngineConfig;
use crate::domain::entities::speculation_report::SpeculationReport;
use crate::domain::error::DomainError;
```
Then the types and logic:
```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AnalyzeArgs {
    /// Ticker symbol, e.g. "AAPL".
    pub ticker: String,
    /// Enable the Reddit source (if no source flags are set, all are enabled).
    pub enable_reddit: Option<bool>,
    pub enable_x: Option<bool>,
    pub enable_bluesky: Option<bool>,
    /// Skip the market snapshot (social-only report).
    pub no_market: Option<bool>,
    /// Posts to fetch per source (default 50).
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct AnalyzeOutput {
    pub summary: String,
    pub report: SpeculationReport,
    pub disclaimer: &'static str,
}

/// Build an AnalysisRequest from the common tool options (shared by all analysis tools).
pub(crate) fn request_from(
    ticker: String,
    enable_reddit: Option<bool>,
    enable_x: Option<bool>,
    enable_bluesky: Option<bool>,
    no_market: Option<bool>,
    limit: Option<usize>,
) -> AnalysisRequest {
    let mut enabled = Vec::new();
    if enable_reddit.unwrap_or(false) {
        enabled.push(SourceKind::Reddit);
    }
    if enable_x.unwrap_or(false) {
        enabled.push(SourceKind::X);
    }
    if enable_bluesky.unwrap_or(false) {
        enabled.push(SourceKind::Bluesky);
    }
    if enabled.is_empty() {
        enabled = SourceKind::ALL.to_vec();
    }
    AnalysisRequest {
        ticker,
        enabled_sources: enabled,
        market_enabled: !no_market.unwrap_or(false),
        limit: limit.unwrap_or(50),
        engine: EngineConfig::default(),
    }
}

/// One-line human gloss for the text-content side of a tool result.
pub(crate) fn summarize(report: &SpeculationReport) -> String {
    format!(
        "{} — {:?} · crowding {:.0}% · {} mentions ({:?})",
        report.ticker.as_str(),
        report.fusion.alignment,
        report.fusion.crowding * 100.0,
        report.social.total_mentions,
        report.social_confidence,
    )
}

pub async fn run_analyze(args: AnalyzeArgs) -> Result<AnalyzeOutput, DomainError> {
    let req = request_from(
        args.ticker,
        args.enable_reddit,
        args.enable_x,
        args.enable_bluesky,
        args.no_market,
        args.limit,
    );
    let report = application::analyze(&req).await?;
    Ok(AnalyzeOutput { summary: summarize(&report), report, disclaimer: DISCLAIMER })
}
```

- [ ] **Step 4: Add the `#[tool]` wrapper** — in `src/mcp/server.rs`, add to the imports `use rmcp::handler::server::wrapper::Parameters;`, and add this method inside the `#[tool_router] impl OpenIntelServer` block:
```rust
    #[tool(
        description = "Analyze one ticker: fuse social sentiment with market action into a \
                       speculation report (net sentiment, speculation index, crowding, \
                       alignment = confirming/diverging/quiet). Read-only — does not trade."
    )]
    async fn analyze_ticker(
        &self,
        Parameters(args): Parameters<tools::AnalyzeArgs>,
    ) -> Result<Json<tools::AnalyzeOutput>, ErrorData> {
        tools::run_analyze(args)
            .await
            .map(Json)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))
    }
```
> `ErrorData::internal_error(message, data)` is the rmcp 2.x error helper (`data: Option<serde_json::Value>`). If its signature differs in the installed version, use the equivalent constructor from the counter example.

- [ ] **Step 5: Run tests + lint**

Run: `cargo test mcp:: && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: PASS (incl. the two new `run_analyze` tests), clean.

- [ ] **Step 6: Commit**

```bash
git add src/mcp/tools.rs src/mcp/server.rs
git commit -m "feat(mcp): add analyze_ticker tool

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 4: `scan_watchlist` tool

**Files:**
- Modify: `src/mcp/tools.rs`, `src/mcp/server.rs`

**Interfaces:**
- Consumes: `request_from`, `application::analyze`, `DISCLAIMER`, `SpeculationReport`.
- Produces: `tools::ScanArgs`, `tools::ScanEntry`, `tools::ScanOutput`, `tools::run_scan(args) -> ScanOutput`.

- [ ] **Step 1: Write the failing test** — append to `src/mcp/tools.rs` `mod tests`:
```rust
    #[tokio::test]
    async fn run_scan_handles_mixed_batch() {
        let out = run_scan(ScanArgs {
            tickers: vec!["AAPL".into(), "$$$".into()],
            enable_reddit: None, enable_x: None, enable_bluesky: None,
            no_market: None, limit: None,
        })
        .await;
        assert_eq!(out.entries.len(), 2);
        assert!(out.entries[0].report.is_some() && out.entries[0].error.is_none());
        assert!(out.entries[1].report.is_none() && out.entries[1].error.is_some());
        assert!(out.disclaimer.contains("Not financial advice"));
    }

    #[tokio::test]
    async fn run_scan_empty_list_is_empty() {
        let out = run_scan(ScanArgs {
            tickers: vec![],
            enable_reddit: None, enable_x: None, enable_bluesky: None,
            no_market: None, limit: None,
        })
        .await;
        assert!(out.entries.is_empty());
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test mcp::tools::tests::run_scan`
Expected: FAIL — `run_scan`/`ScanArgs` not found.

- [ ] **Step 3: Implement** — add to `src/mcp/tools.rs` (above the tests):
```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScanArgs {
    /// Ticker symbols to analyze concurrently.
    pub tickers: Vec<String>,
    pub enable_reddit: Option<bool>,
    pub enable_x: Option<bool>,
    pub enable_bluesky: Option<bool>,
    pub no_market: Option<bool>,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct ScanEntry {
    pub ticker: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<SpeculationReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ScanOutput {
    pub entries: Vec<ScanEntry>,
    pub disclaimer: &'static str,
}

pub async fn run_scan(args: ScanArgs) -> ScanOutput {
    let ScanArgs { tickers, enable_reddit, enable_x, enable_bluesky, no_market, limit } = args;
    let futures = tickers.into_iter().map(|t| async move {
        let req = request_from(t.clone(), enable_reddit, enable_x, enable_bluesky, no_market, limit);
        match application::analyze(&req).await {
            Ok(report) => ScanEntry { ticker: t, report: Some(report), error: None },
            Err(e) => ScanEntry { ticker: t, report: None, error: Some(e.to_string()) },
        }
    });
    let entries = futures::future::join_all(futures).await;
    ScanOutput { entries, disclaimer: DISCLAIMER }
}
```

- [ ] **Step 4: Add the `#[tool]` wrapper** — in `src/mcp/server.rs`, inside the `#[tool_router] impl`:
```rust
    #[tool(
        description = "Analyze a watchlist of tickers concurrently. Returns one entry per \
                       ticker (report or error); one bad ticker does not fail the batch. \
                       Read-only — does not trade."
    )]
    async fn scan_watchlist(
        &self,
        Parameters(args): Parameters<tools::ScanArgs>,
    ) -> Result<Json<tools::ScanOutput>, ErrorData> {
        Ok(Json(tools::run_scan(args).await))
    }
```

- [ ] **Step 5: Run tests + lint**

Run: `cargo test mcp:: && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: PASS, clean.

- [ ] **Step 6: Commit**

```bash
git add src/mcp/tools.rs src/mcp/server.rs
git commit -m "feat(mcp): add scan_watchlist tool

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 5: `compare_tickers` tool

**Files:**
- Modify: `src/mcp/tools.rs`, `src/mcp/server.rs`

**Interfaces:**
- Consumes: `request_from`, `application::analyze`, `DISCLAIMER`, `SpeculationReport`, `Alignment`.
- Produces: `tools::RankBy`, `tools::CompareArgs`, `tools::RankedEntry`, `tools::CompareError`, `tools::CompareOutput`, `tools::rank_metric`, `tools::sort_ranked`, `tools::run_compare(args) -> CompareOutput`.

- [ ] **Step 1: Write the failing tests** — append to `src/mcp/tools.rs` `mod tests`. The first exercises `sort_ranked` with reports built via the engine (so metrics genuinely differ); the second exercises `run_compare` plumbing/error-partitioning.
```rust
    #[tokio::test]
    async fn sort_ranked_orders_by_crowding_desc() {
        use crate::domain::engine::config::EngineConfig;
        use crate::domain::engine::speculation_engine::SpeculationEngine;
        use crate::domain::entities::social_post::{PostText, SocialPost};
        use crate::domain::entities::ticker::Ticker;
        use crate::domain::values::polarity::Polarity;
        use crate::domain::values::post_signal::PostSignal;
        use chrono::{TimeZone, Utc};

        let t = Ticker::parse("AAPL").unwrap();
        let now = Utc.with_ymd_and_hms(2026, 6, 29, 0, 0, 0).unwrap();
        let post = SocialPost {
            id: "1".into(),
            source: SourceKind::Reddit,
            author: "a".into(),
            text: PostText::parse("x").unwrap(),
            created_at: now,
            engagement: 0,
        };
        // high crowding: speculative post; low crowding: non-speculative.
        let hi = SpeculationEngine::aggregate(
            &t, &[post.clone()], &[PostSignal { polarity: Polarity::new(0.0), speculative: true }],
            None, now, &EngineConfig::default()).unwrap();
        let lo = SpeculationEngine::aggregate(
            &t, &[post.clone()], &[PostSignal { polarity: Polarity::new(0.0), speculative: false }],
            None, now, &EngineConfig::default()).unwrap();
        assert!(hi.fusion.crowding > lo.fusion.crowding);

        let mut ranked = vec![
            RankedEntry { ticker: "LO".into(), rank_metric: lo.fusion.crowding, report: lo },
            RankedEntry { ticker: "HI".into(), rank_metric: hi.fusion.crowding, report: hi },
        ];
        sort_ranked(&mut ranked, RankBy::Crowding);
        assert_eq!(ranked[0].ticker, "HI");
        assert_eq!(ranked[1].ticker, "LO");
    }

    #[tokio::test]
    async fn run_compare_partitions_valid_and_invalid() {
        let out = run_compare(CompareArgs {
            tickers: vec!["AAPL".into(), "$$$".into()],
            rank_by: RankBy::Crowding,
            enable_reddit: None, enable_x: None, enable_bluesky: None,
            no_market: None, limit: None,
        })
        .await;
        assert_eq!(out.ranked.len(), 1);
        assert_eq!(out.errors.len(), 1);
        assert_eq!(out.errors[0].ticker, "$$$");
        assert!(out.ranked[0].rank_metric.is_finite());
    }
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test mcp::tools::tests::sort_ranked mcp::tools::tests::run_compare`
Expected: FAIL — `RankBy`/`run_compare`/`sort_ranked` not found.

- [ ] **Step 3: Implement** — add to `src/mcp/tools.rs` (above the tests). Add `use crate::domain::values::speculation::Alignment;` near the other imports.
```rust
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RankBy {
    /// Blended crowding score (default).
    #[default]
    Crowding,
    SpeculationIndex,
    NetSentiment,
    /// Diverging tickers first, then by crowding.
    Divergence,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CompareArgs {
    pub tickers: Vec<String>,
    #[serde(default)]
    pub rank_by: RankBy,
    pub enable_reddit: Option<bool>,
    pub enable_x: Option<bool>,
    pub enable_bluesky: Option<bool>,
    pub no_market: Option<bool>,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct RankedEntry {
    pub ticker: String,
    pub rank_metric: f64,
    pub report: SpeculationReport,
}

#[derive(Debug, Serialize)]
pub struct CompareError {
    pub ticker: String,
    pub error: String,
}

#[derive(Debug, Serialize)]
pub struct CompareOutput {
    pub rank_by: RankBy,
    pub ranked: Vec<RankedEntry>,
    pub errors: Vec<CompareError>,
    pub disclaimer: &'static str,
}

fn rank_metric(report: &SpeculationReport, rank_by: RankBy) -> f64 {
    match rank_by {
        // `divergence` ranks categorically (diverging first) then by crowding,
        // so its numeric metric is crowding.
        RankBy::Crowding | RankBy::Divergence => report.fusion.crowding,
        RankBy::SpeculationIndex => report.social.speculation_index.value(),
        RankBy::NetSentiment => report.social.net_sentiment.value(),
    }
}

pub(crate) fn sort_ranked(ranked: &mut [RankedEntry], rank_by: RankBy) {
    ranked.sort_by(|a, b| {
        if matches!(rank_by, RankBy::Divergence) {
            let a_div = matches!(a.report.fusion.alignment, Alignment::Diverging);
            let b_div = matches!(b.report.fusion.alignment, Alignment::Diverging);
            b_div.cmp(&a_div).then_with(|| {
                b.rank_metric.partial_cmp(&a.rank_metric).unwrap_or(std::cmp::Ordering::Equal)
            })
        } else {
            b.rank_metric.partial_cmp(&a.rank_metric).unwrap_or(std::cmp::Ordering::Equal)
        }
    });
}

pub async fn run_compare(args: CompareArgs) -> CompareOutput {
    let CompareArgs { tickers, rank_by, enable_reddit, enable_x, enable_bluesky, no_market, limit } =
        args;
    let futures = tickers.into_iter().map(|t| async move {
        let req = request_from(t.clone(), enable_reddit, enable_x, enable_bluesky, no_market, limit);
        (t, application::analyze(&req).await)
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

- [ ] **Step 4: Add the `#[tool]` wrapper** — in `src/mcp/server.rs`, inside the `#[tool_router] impl`:
```rust
    #[tool(
        description = "Compare tickers and rank them by a chosen signal: rank_by ∈ \
                       {crowding (default), speculation_index, net_sentiment, divergence}. \
                       Read-only — does not trade."
    )]
    async fn compare_tickers(
        &self,
        Parameters(args): Parameters<tools::CompareArgs>,
    ) -> Result<Json<tools::CompareOutput>, ErrorData> {
        Ok(Json(tools::run_compare(args).await))
    }
```

- [ ] **Step 5: Run the full suite + lint**

Run: `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: PASS (all prior + the new compare tests), clean.

- [ ] **Step 6: Commit**

```bash
git add src/mcp/tools.rs src/mcp/server.rs
git commit -m "feat(mcp): add compare_tickers tool with ranking

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 6: README — use with an AI agent

**Files:**
- Modify: `README.md`

**Interfaces:** none (docs).

- [ ] **Step 1: Add an MCP section** — insert after the existing "Usage" section in `README.md`:

````markdown
## Use with an AI agent (MCP)

OpenIntel can run as a local **MCP server** so an AI agent can consult its analysis while
you trade through **Robinhood's official Agentic Trading MCP**. OpenIntel is the
intelligence layer; the agent is the brain; Robinhood's MCP is execution.

```
your agent (Claude Code on your subscription / ChatGPT / Codex / Cursor / Grok)
  ├─ MCP → openintel                          (analysis — this tool)
  └─ MCP → agent.robinhood.com/mcp/trading    (execution, sandboxed agentic wallet)
```

Wire it up (Claude Code shown; other agents add the same `openintel mcp` stdio command in
their MCP settings):

```bash
cargo install --path .          # puts `openintel` on your PATH
claude mcp add openintel -- openintel mcp
```

Tools exposed (all **read-only** — OpenIntel never places trades):

| Tool | What it does |
|---|---|
| `analyze_ticker` | One symbol → full speculation report (sentiment, speculation index, crowding, alignment) |
| `scan_watchlist` | A list of symbols → reports, run concurrently |
| `compare_tickers` | Rank a set by `crowding` / `speculation_index` / `net_sentiment` / `divergence` |
| `list_sources` | Which data sources are available |

### Safety

OpenIntel is a **screener, not financial advice**, and it **cannot execute trades** — that
boundary is by design. When you connect a trading MCP:

- Keep the broker's **approval-required** mode on; don't authorize unattended execution.
- Fund a deliberately **small agentic wallet** — that balance is your blast-radius cap.
- Treat the agent's reads of your accounts as a privacy surface, and the analysis as one
  signal among many. AI agents err; you are responsible for every trade placed.
````

- [ ] **Step 2: Confirm the tree is clean and commit**

Run: `git status --short` (only `README.md` modified), then:
```bash
git add README.md
git commit -m "docs: README section for the MCP agent surface + safety

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage:** §3 application extraction + `AnalysisRequest` + `SourceKind::ALL` + shared `DISCLAIMER` → Task 1. §4 four tools (`analyze_ticker`/`scan_watchlist`/`list_sources`/`compare_tickers`) with exact I/O + `compare` ranking incl. divergence → Tasks 2–5. §5 `Json<Output>` + summary format + disclaimer field → Tasks 3–5 (`AnalyzeOutput.summary`, `summarize`). §6 rmcp 2.x (`transport-io`, `serve(stdio())`, `Result<Json<_>, ErrorData>`), stdout discipline (no `println!`, no `tracing`) → Task 2. §6 `openintel mcp` subcommand → Task 2. §7 tests: logic fns split from wrappers, asserted on typed structs; empty/dup batch behavior (`run_scan([])`) → Tasks 3–5. §8 analysis-only boundary + safety guidance → enforced by omission (no broker code) + Task 6 README.

**Placeholder scan:** No `TODO`/`TBD`/"handle errors appropriately". Every code step shows complete code; the two rmcp-API notes are explicit "if the installed version differs, mirror the linked counter example" instructions, not placeholders — the primary code is concrete.

**Type consistency:** `application::analyze(&AnalysisRequest) -> Result<SpeculationReport, DomainError>` identical across Task 1 def and Tasks 3–5 call sites. `request_from(...)` (Task 3) reused verbatim by `run_scan`/`run_compare` (Tasks 4–5). `Json<_>` + `ErrorData` return shape identical across all four `#[tool]` wrappers. `SourceKind::ALL` used in Task 1 (`AppConfig::new`, `application` tests) and Task 2 (`run_list_sources`) and Task 3 (`request_from`). `DISCLAIMER` single const (Task 1) referenced by `cli/run.rs` and every analysis tool output. `RankBy` variants consistent between the enum (Task 5), `rank_metric`, `sort_ranked`, and the tool description.
