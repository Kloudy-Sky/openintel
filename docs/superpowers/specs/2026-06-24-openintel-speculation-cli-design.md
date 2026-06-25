# OpenIntel — Social + Market Speculation Intelligence CLI

**Status:** Reviewed — must-fix + polish applied (A–J)
**Date:** 2026-06-24
**Type:** Greenfield rewrite (full teardown of existing `src/`)

---

## 1. Thesis & framing

OpenIntel is a **security-first, open-source CLI** that ingests social-media chatter and
market data for a ticker and produces a **speculation report** — a fusion of crowd
sentiment with price action.

It is **not** a buy/sell oracle. Sentiment alone is noisy, easily manipulated, and mostly
coincident-to-lagging price. The defensible product is a **crowding & divergence detector**:

> Surface when retail speculation and price/market action **confirm** each other
> (crowded, fragile, blow-off risk) or **diverge** (sentiment up, price down → trap/reversal).

This is a **screener that informs**, explicitly **not financial advice**. Every report
carries that disclaimer, and low-sample reports are confidence-gated.

Key insight driving the architecture: **speculation is measured more truthfully in the
market than in tweets.** Social jargon shows the crowd is *loud*; the options chain shows
the crowd is *leveraged*. The engine fuses both.

---

## 2. Scope

### v1 (this spec)
- Hexagonal scaffold: `domain / adapters / config / cli`.
- Two data-source ports + one swappable analyzer port.
- **Mocked adapters only** (deterministic fixtures): `mock_reddit`, `mock_x`,
  `mock_bluesky`, `mock_market`.
- Real offline `LexiconAnalyzer`.
- Pure fusion engine producing a `SpeculationReport`.
- Env-only secret loading via `secrecy` (built now, unused by mocks).
- `analyze` CLI command with source toggles + `table|json` output.
- Unit + integration tests.

### Explicitly deferred (future specs — *designed for, not built now*)
- Real network adapters (`reqwest` + `rustls`): Reddit OAuth, X API, Bluesky AT Protocol, Yahoo Finance.
- **Mention velocity / history** — the *spike* in mentions is the real signal; requires persistence (deferred).
- Options-chain depth: put/call ratio, IV rank, unusual options (typed as `Option<_>` now).
- Bot / quality filtering (manipulation defense).
- Multiple market-provider selection; TOML config file.

### Non-goals
- No persistence/DB in v1. No real HTTP. No trade execution. No portfolio tracking.

---

## 3. Architecture

Pure hexagonal. **Domain is pure and sync** (no `tokio`, no IO). Ports are async *trait
definitions* so adapters can do real IO later. The async fan-out lives at the edge
(composition root in `cli::run`). The scoring/fusion engine never touches a port — it
receives already-fetched, already-scored data, so it is fully deterministic and unit-testable.

```
src/
  main.rs                     # thin binary → cli::run
  lib.rs                      # module tree; exposes testable run()
  domain/
    entities/
      ticker.rs               # Ticker — validated symbol
      social_post.rs          # SocialPost (+ PostText newtype)
      market_snapshot.rs      # MarketSnapshot — price/volume/(options)
      speculation_report.rs   # SpeculationReport — the fused engine output
    values/
      source_kind.rs          # SourceKind { Reddit, X, Bluesky }
      polarity.rs             # Polarity — [-1.0, 1.0]
      speculation.rs          # SpeculationIndex [0,1], Confidence, Alignment
    ports/
      social_data_source.rs   # #[async_trait] SocialDataSource → Vec<SocialPost>
      market_data_source.rs   # #[async_trait] MarketDataSource → MarketSnapshot
      post_analyzer.rs        # #[async_trait] PostAnalyzer (the swappable text→signal plug)
    engine/
      speculation_engine.rs   # PURE sync: posts + signals + market + now → SpeculationReport
      config.rs               # EngineConfig (thresholds, weights) — pure domain knobs
    error.rs                  # DomainError (thiserror)
  adapters/
    sources/
      mock_reddit.rs · mock_x.rs · mock_bluesky.rs   # impl SocialDataSource
    market/
      mock_market.rs          # impl MarketDataSource
    analyzer/
      lexicon.rs              # LexiconAnalyzer — offline word lists, impl PostAnalyzer
  config/
    settings.rs               # AppConfig — non-secret runtime config from CLI
    secrets.rs                # Credentials — secrecy::SecretString from env only
  cli/
    args.rs                   # clap Parser/Subcommand
    run.rs                    # orchestration: enabled sources → concurrent fetch
                              #   → analyzer → engine → render
tests/                        # integration tests over run()
```

---

## 4. Domain model

### Entities & value objects

```
Ticker(String)            // validated: 1–5 A–Z, optional .CLASS (e.g. BRK.B); uppercased
PostText(String)          // non-empty, length-bounded
SourceKind                // enum { Reddit, X, Bluesky }
Polarity(f64)             // constructor clamps to [-1.0, 1.0]
SpeculationIndex(f64)     // constructor clamps to [0.0, 1.0]
Confidence                // enum { Low, Medium, High } — measures SOCIAL SAMPLE ADEQUACY only
Alignment                 // enum { ConfirmingBullish, ConfirmingBearish, Diverging, Quiet }
PostSignal {              // per-post analyzer output: sentiment + speculation flag
  polarity: Polarity, speculative: bool,
}

SocialPost {
  id: String, source: SourceKind, author: String,
  text: PostText, created_at: DateTime<Utc>, engagement: u32,  // engagement stored, unused in v1
}

MarketSnapshot {
  ticker: Ticker, as_of: DateTime<Utc>,
  last_price: f64, previous_close: f64, volume: u64, avg_volume: u64,
  realized_vol: Option<f64>, put_call_ratio: Option<f64>, iv_rank: Option<f64>,  // future-typed
}
```

### Report (engine output)

```
SpeculationReport {
  ticker, generated_at,              // generated_at injected by the caller (engine stays pure)
  social: SocialSummary,
  market: Option<MarketSummary>,     // None when --no-market
  fusion: FusionSignals,
  social_confidence: Confidence,     // social sample adequacy ONLY (ignores market-data quality)
}
// The "Not financial advice" disclaimer is appended by the renderer, not stored on the entity.

SocialSummary {
  total_mentions, mentions_by_source: BTreeMap<SourceKind, usize>,
  net_sentiment: Polarity, bullish: usize, bearish: usize, neutral: usize,
  bull_bear_ratio: Option<f64>,      // None when bearish == 0 (no divide-by-zero lie)
  speculation_index: SpeculationIndex,
}

MarketSummary {
  last_price, pct_change: f64, rvol: f64,
  realized_vol: Option<f64>, put_call_ratio: Option<f64>, iv_rank: Option<f64>,
}

FusionSignals { alignment: Alignment, crowding: f64 /* [0,1] */, notes: Vec<String> }
```

### Ports

```rust
#[async_trait]
trait SocialDataSource {
    fn kind(&self) -> SourceKind;
    async fn fetch(&self, ticker: &Ticker, limit: usize) -> Result<Vec<SocialPost>, DomainError>;
}

#[async_trait]
trait MarketDataSource {
    fn name(&self) -> &'static str;
    async fn snapshot(&self, ticker: &Ticker) -> Result<MarketSnapshot, DomainError>;
}

#[async_trait]
trait PostAnalyzer {
    /// Returns one PostSignal per input post, aligned to input order (len == posts.len()).
    /// Owns ALL text understanding — sentiment AND speculation-jargon — so the engine holds no lexicon.
    async fn analyze(&self, posts: &[SocialPost]) -> Result<Vec<PostSignal>, DomainError>;
}
```

`PostAnalyzer` is async **now** so an `LlmAnalyzer` (or batched ML) drops in later without a
breaking port change. The lexicon impl does sync work inside the async fn. Keeping all word
lists in the adapter is what lets the engine stay a pure aggregator.

### Engine (pure, sync — the deterministic core)

```rust
impl SpeculationEngine {
    fn aggregate(
        ticker: &Ticker,
        posts: &[SocialPost],
        signals: &[PostSignal],      // from the analyzer; MUST equal posts.len() (validated)
        market: Option<&MarketSnapshot>,
        now: DateTime<Utc>,          // injected — engine never reads the clock (stays pure/deterministic)
        cfg: &EngineConfig,
    ) -> Result<SpeculationReport, DomainError>;  // Err(AnalyzerMismatch) if signals.len() != posts.len()
}
```

---

## 5. Metrics & fusion (concrete formulas)

### Analyzer-side (LexiconAnalyzer internals — produces one `PostSignal` per post)
- **polarity** = `(bull_hits − bear_hits) / (bull_hits + bear_hits)`, else `0` (clamped to `[-1, 1]`).
- **speculative** = text contains ≥1 options/leverage term — jargon set
  (`calls`, `puts`, `0dte`, `yolo`, `leaps`, `theta`, `gamma`, `squeeze`, `otm`, `itm`, strike notation, …).

The word lists live **only** in the adapter; the engine never sees them.

### Engine-side (pure aggregation over `&[PostSignal]`)
A post is **bullish** if `polarity > +τ`, **bearish** if `< −τ`, else **neutral** (`τ` = 0.2 default).

**Social:**
- `net_sentiment` = mean of `signal.polarity`. **Empty input → 0** (no `0/0`).
- `bull_bear_ratio` = `bullish / bearish`, or `None` if `bearish == 0`.
- `speculation_index` = `count(signal.speculative) / total_posts`. **Empty input → 0.**

**Market** (only when a snapshot is present):
- `pct_change` = `(last_price − previous_close) / previous_close * 100`.
- `rvol` = `volume / avg_volume`; `avg_volume == 0` → `rvol = 0` + note.

**Fusion — `crowding` ∈ [0,1]:** weighted blend of the *available* components, **renormalized**
over present weights so a missing input never silently deflates the score:
- components: `speculation_index` (w=0.5), `min(rvol / rvol_cap, 1)` (w=0.3), `iv_rank` (w=0.2).
- `crowding = Σ(wᵢ · valueᵢ) / Σ(wᵢ)` over only the components that exist (market and IV may be absent).
- no components at all (no market, no posts) → `crowding = 0`.

**Fusion — `alignment`:**
- `market` absent (`--no-market` / fetch failed) → `Quiet` + "social-only, no price reference" note.
- `total_mentions < min_sample` → `Quiet`.
- else compare `net_sentiment` vs `pct_change`, gated by the **aggregate** `net_sentiment_threshold σ`
  (default `0.05`) and price move threshold `δ` (default `1.0%`). `σ` is distinct from the per-post `τ`
  — a *mean* polarity sits near 0, so it needs its own, smaller threshold:
  - both meaningful, same sign → `ConfirmingBullish` / `ConfirmingBearish`.
  - both meaningful, opposite sign → `Diverging`.
  - otherwise → `Quiet`.

**`social_confidence`** by `total_mentions`: Low `< 10`, Medium `10–49`, High `≥ 50`.
Measures social sample adequacy only — *not* market-data quality.

### EngineConfig (pure domain knobs, non-secret) — `domain/engine/config.rs`
`bull_bear_threshold τ=0.2`, `net_sentiment_threshold σ=0.05`, `price_move_threshold δ=1.0`,
`crowding_weights=(spec 0.5, rvol 0.3, iv 0.2)`, `rvol_cap=3.0`, `min_sample=10`,
confidence thresholds `(10, 50)`. Sensible defaults; overridable.

---

## 6. Config & security posture

- **Secrets: env-only, never on disk.** `Credentials::from_env()` loads
  `OPENINTEL_REDDIT_TOKEN`, `OPENINTEL_X_BEARER`, `OPENINTEL_BLUESKY_APP_PASSWORD`,
  `OPENINTEL_MARKET_API_KEY` into `secrecy::SecretString` (zeroized on drop, redacted `Debug`,
  never logged). All optional in v1 (mocks need none); the redacting loader exists so real
  adapters drop in cleanly.
- **Non-secret config** (toggles, limits, format) via CLI flags only — no TOML in v1 (YAGNI).
- **Output disclaimer**: the renderer appends "Not financial advice." to every report (table and json).
- **Input validation at the boundary**: `Ticker::parse` rejects malformed symbols.
- Future network adapters will use `rustls` (no OpenSSL).

---

## 7. CLI surface

```
openintel analyze <TICKER>
    [--enable-reddit] [--enable-x] [--enable-bluesky]   # social toggles; none given => all enabled
    [--no-market]                                        # skip the market snapshot (social-only report)
    [--limit <N>]                                        # posts per source (default 50)
    [--format table|json]                                # default: table
```

- Market snapshot **on by default** (single mock provider); `--no-market` yields `market: None`.
- `table` = human readout; `json` = machine-readable (serde). The renderer appends the disclaimer to both.

---

## 8. Execution flow

```
#[tokio::main] main
  → Cli::parse()
  → AppConfig::load(args) + Credentials::from_env()
  → cli::run(cfg):
      build Vec<Box<dyn SocialDataSource>> from toggles      // registry
      build market source (unless --no-market)
      futures::join_all(source.fetch(ticker, limit))         // concurrent, mocked
        → per-source Err is NON-FATAL: drop that source, push a note, keep the rest
      market.snapshot(ticker).await                          // concurrent; Err → market = None + note
      analyzer.analyze(&all_posts).await                     // the swappable text→signal plug
      let now = Utc::now();                                  // clock read HERE, at the edge — never in the engine
      SpeculationEngine::aggregate(ticker, &posts, &signals, market.as_ref(), now, &engine_cfg)?
      render(report, format)                                 // table | json; renderer appends disclaimer
```

If **all** social sources fail **and** there is no market data, `run` returns an error rather than
emitting a hollow report.

Demonstrable injection path: **mock adapter → analyzer port → pure engine → output.**

---

## 9. Dependencies (`Cargo.toml`)

Lean — only what v1 uses:

| Crate | Purpose |
|---|---|
| `tokio` (`rt-multi-thread`, `macros`) | async runtime |
| `clap` (`derive`) | CLI parsing |
| `serde`, `serde_json` | JSON output / future deserialization |
| `secrecy` | secret API-key handling |
| `async-trait` | async port traits |
| `thiserror` | domain error types |
| `chrono` (`serde`) | timestamps |
| `futures` | `join_all` concurrent fetch |

**Deliberately omitted until the first real adapter:** `reqwest`, `rusqlite`, `uuid`
(v1 has no network/DB — adding them now is dead weight).

---

## 10. Testing strategy

- **Unit:**
  - `Ticker::parse` — valid (`AAPL`, `BRK.B`) / invalid (empty, too long, symbols).
  - `LexiconAnalyzer` — `PostSignal` for bullish / bearish / neutral / jargon-bearing text
    (asserts both `polarity` and `speculative`).
  - `SpeculationEngine::aggregate` — exact report from fixed `PostSignal`s + injected `now`;
    **confirming** and **diverging** fixtures; `bull_bear_ratio == None` edge; `avg_volume == 0`
    rvol guard; **empty input** (0 posts → zeros, `Quiet`, `Low`); **no-market** → `Quiet`;
    `signals.len() != posts.len()` → `Err(AnalyzerMismatch)`; crowding renormalization (IV present vs absent).
- **Integration (`tests/`):**
  - `run()` over mock social + mock market → assert `SpeculationReport` (via JSON).
  - `--no-market` path → `market: None`, `alignment = Quiet`.
  - One social source fails → report still produced from the rest, with a note (non-fatal).
  - Deterministic: mocks return fixed fixtures **including fixed timestamps**.

---

## 11. Extensibility playbook (documented in README)

**Add a social source** (e.g. real Reddit): new struct in `adapters/sources/`,
`impl SocialDataSource`, add a `SourceKind` variant, one line in the `cli::run` builder.

**Add a market source** (e.g. Yahoo Finance): new struct in `adapters/market/`,
`impl MarketDataSource`, register in the builder.

**Swap the analyzer** (lexicon → LLM/ML): new struct in `adapters/analyzer/`,
`impl PostAnalyzer`. No engine change.

The pattern is identical across all three; types stay honest.

---

## 12. Risks & responsible use

- **Manipulation:** social sentiment is gameable (bots, coordinated pumps). v1 mitigates only
  via confidence-gating; bot filtering is a deferred priority.
- **Signal nature:** sentiment is coincident-to-lagging; treat as *context*, not prediction.
- **Low sample:** small mention counts are unreliable → `social_confidence = Low` + disclaimer.
- **Responsible framing:** screener that informs, **not financial advice** — surfaced in every report.

---

## 13. Decisions log

1. Greenfield teardown (wipe `src/`), not evolve-on-existing.
2. Text-analysis **port `PostAnalyzer`** (swappable lexicon → AI), async now, emits `PostSignal`
   (sentiment + speculation) — all word lists stay in the adapter.
3. **Two focused data-source ports** (`SocialDataSource`, `MarketDataSource`) — no god-type.
4. Report renamed `SentimentReport` → **`SpeculationReport`** (fuses social + market).
5. Env-only secrets, no secrets file. Default: all social sources on, market on.
6. Velocity/history is the top deferred item (needs persistence).
7. Engine is pure: `now` is injected, `aggregate` returns `Result` and validates `signals.len()`.
8. Per-source fetch failure is non-fatal; all-fail **and** no-market → error, not an empty report.
9. `crowding` renormalizes over present components; alignment uses aggregate `σ`, distinct from per-post `τ`.
10. Disclaimer lives in the renderer; `confidence` → `social_confidence` (sample adequacy only).
