# OpenIntel Speculation CLI — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a security-first Rust CLI that ingests mocked social + market data for a ticker and fuses crowd sentiment with price action into a `SpeculationReport` (crowding & divergence detector).

**Architecture:** Hexagonal / ports-and-adapters. A pure, synchronous domain (entities, value objects, a deterministic `SpeculationEngine`) sits behind async port traits (`SocialDataSource`, `MarketDataSource`, `PostAnalyzer`). Adapters (mock sources, offline `LexiconAnalyzer`) implement the ports. The async fan-out and the clock live only at the edge (`cli::run` / `main`), so the engine stays 100% deterministic and unit-testable.

**Tech Stack:** Rust 2021 (rustc 1.92), Tokio, Clap (derive), Serde/serde_json, Secrecy, async-trait, thiserror, chrono, futures.

**Spec:** `docs/superpowers/specs/2026-06-24-openintel-speculation-cli-design.md`

## Global Constraints

- **Edition:** `2021`. **Crate name:** `openintel` (lib + bin).
- **Dependency versions (verified current 2026-06-24):** `tokio = "1"` (features `rt-multi-thread`, `macros`), `clap = "4"` (feature `derive`), `serde = "1"` (feature `derive`), `serde_json = "1"`, `secrecy = "0.10"`, `async-trait = "0.1"`, `thiserror = "2"`, `chrono = "0.4"` (feature `serde`), `futures = "0.3"`.
- **No network, no database in v1.** Do NOT add `reqwest`, `rusqlite`, or `uuid`.
- **Secrets are env-only**, wrapped in `secrecy::SecretString`, never logged or written to disk. `secrecy 0.10` API: construct via `SecretString::new(value.into_boxed_str())`; read via `ExposeSecret::expose_secret(&self) -> &str`.
- **Domain is pure & sync:** no `tokio`, no IO, no `Utc::now()`, no `std::env` inside `src/domain/`. The clock is injected.
- **Disclaimer** ("Not financial advice…") is appended by the renderer, never stored on the domain entity.
- **Every task ends green:** `cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` must pass before the commit step.
- **Commit style:** concise, conventional-commit prefix; end body with `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`.

---

## File Structure

```
Cargo.toml                                   # deps + bin/lib config
src/
  main.rs                                    # composition root (Task 16)
  lib.rs                                     # module tree (Task 1)
  domain/
    mod.rs  error.rs                         # DomainError (Task 2)
    values/{mod,polarity,speculation,source_kind,post_signal}.rs   # (Task 3)
    entities/{mod,ticker,social_post,market_snapshot,speculation_report}.rs  # (Tasks 4-6)
    ports/{mod,social_data_source,market_data_source,post_analyzer}.rs       # (Task 7)
    engine/{mod,config,speculation_engine}.rs                                # (Tasks 8-9)
  adapters/
    mod.rs
    analyzer/{mod,lexicon}.rs                # LexiconAnalyzer (Task 10)
    sources/{mod,mock_reddit,mock_x,mock_bluesky}.rs    # (Task 11)
    market/{mod,mock_market}.rs              # (Task 12)
  config/{mod,secrets,settings}.rs           # Credentials + AppConfig (Task 13)
  cli/{mod,args,run}.rs                      # clap + orchestration + render (Tasks 14-15)
tests/analyze_flow.rs                        # integration (Task 17)
README.md                                    # usage + extensibility (Task 18)
```

---

### Task 1: Teardown + compiling skeleton

Wipe the old project and stand up an empty-but-compiling module tree, so every later task only fills one leaf file.

**Files:**
- Delete: everything under `src/` and `tests/` (old knowledge-base code), plus stray `Icon\r` files.
- Create: `Cargo.toml`, `src/lib.rs`, `src/main.rs`, and the full module tree (mod files populated, leaf files empty).
- Modify: `.gitignore`.

**Interfaces:**
- Consumes: nothing.
- Produces: a compiling crate `openintel` with public modules `domain`, `adapters`, `config`, `cli`.

- [ ] **Step 1: Remove old source, tests, and Finder junk**

```bash
git rm -r --quiet src tests
rm -f Cargo.lock
find . -name 'Icon?' -not -path './.git/*' -print -delete   # the literal "Icon\r" macOS artifacts
```

- [ ] **Step 2: Add Finder-junk ignore rule**

Append to `.gitignore`:

```gitignore
# macOS Finder artifacts
Icon?
.DS_Store
```

- [ ] **Step 3: Write `Cargo.toml`**

```toml
[package]
name = "openintel"
version = "0.1.0"
edition = "2021"
description = "Security-first CLI that fuses social sentiment with market action into a speculation report"
license = "MIT"

[[bin]]
name = "openintel"
path = "src/main.rs"

[lib]
name = "openintel"
path = "src/lib.rs"

[dependencies]
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
secrecy = "0.10"
async-trait = "0.1"
thiserror = "2"
chrono = { version = "0.4", features = ["serde"] }
futures = "0.3"
```

- [ ] **Step 4: Write `src/lib.rs`**

```rust
pub mod adapters;
pub mod cli;
pub mod config;
pub mod domain;
```

- [ ] **Step 5: Write `src/main.rs` (temporary minimal binary)**

```rust
fn main() {}
```

- [ ] **Step 6: Create the module tree (mod files populated; leaf files empty)**

Create these files with the given contents:

`src/domain/mod.rs`
```rust
pub mod engine;
pub mod entities;
pub mod error;
pub mod ports;
pub mod values;
```
`src/domain/values/mod.rs`
```rust
pub mod polarity;
pub mod post_signal;
pub mod source_kind;
pub mod speculation;
```
`src/domain/entities/mod.rs`
```rust
pub mod market_snapshot;
pub mod social_post;
pub mod speculation_report;
pub mod ticker;
```
`src/domain/ports/mod.rs`
```rust
pub mod market_data_source;
pub mod post_analyzer;
pub mod social_data_source;
```
`src/domain/engine/mod.rs`
```rust
pub mod config;
pub mod speculation_engine;
```
`src/adapters/mod.rs`
```rust
pub mod analyzer;
pub mod market;
pub mod sources;
```
`src/adapters/analyzer/mod.rs`
```rust
pub mod lexicon;
```
`src/adapters/sources/mod.rs`
```rust
pub mod mock_bluesky;
pub mod mock_reddit;
pub mod mock_x;
```
`src/adapters/market/mod.rs`
```rust
pub mod mock_market;
```
`src/config/mod.rs`
```rust
pub mod secrets;
pub mod settings;
```
`src/cli/mod.rs`
```rust
pub mod args;
pub mod run;
```

Create these leaf files **empty** (0 bytes — they become valid empty modules, filled in later tasks):
```
src/domain/error.rs
src/domain/values/{polarity,post_signal,source_kind,speculation}.rs
src/domain/entities/{market_snapshot,social_post,speculation_report,ticker}.rs
src/domain/ports/{market_data_source,post_analyzer,social_data_source}.rs
src/domain/engine/{config,speculation_engine}.rs
src/adapters/analyzer/lexicon.rs
src/adapters/sources/{mock_bluesky,mock_reddit,mock_x}.rs
src/adapters/market/mock_market.rs
src/config/{secrets,settings}.rs
src/cli/{args,run}.rs
```

- [ ] **Step 7: Verify it compiles**

Run: `cargo build && cargo test`
Expected: builds clean; `running 0 tests`.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "feat: teardown old code, scaffold hexagonal module tree

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 2: DomainError

**Files:**
- Modify: `src/domain/error.rs`

**Interfaces:**
- Produces: `pub enum DomainError` with variants `InvalidTicker(String)`, `InvalidPostText(String)`, `AnalyzerMismatch { expected: usize, got: usize }`, `SourceFailure { source: String, message: String }`, `NoData`. Implements `std::error::Error + Display + Debug` (via thiserror).

- [ ] **Step 1: Write the failing test** — append to `src/domain/error.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displays_human_messages() {
        assert_eq!(DomainError::InvalidTicker("@@".into()).to_string(), "invalid ticker: @@");
        assert_eq!(
            DomainError::AnalyzerMismatch { expected: 3, got: 2 }.to_string(),
            "analyzer returned 2 signals for 3 posts"
        );
        assert_eq!(DomainError::NoData.to_string(), "no data: all sources failed");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p openintel domain::error`
Expected: FAIL — `DomainError` not found.

- [ ] **Step 3: Write the implementation** — prepend above the test module in `src/domain/error.rs`

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DomainError {
    #[error("invalid ticker: {0}")]
    InvalidTicker(String),

    #[error("invalid post text: {0}")]
    InvalidPostText(String),

    #[error("analyzer returned {got} signals for {expected} posts")]
    AnalyzerMismatch { expected: usize, got: usize },

    #[error("data source '{source}' failed: {message}")]
    SourceFailure { source: String, message: String },

    #[error("no data: all sources failed")]
    NoData,
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test domain::error && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: PASS, no clippy/fmt issues.

- [ ] **Step 5: Commit**

```bash
git add src/domain/error.rs
git commit -m "feat(domain): add DomainError

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 3: Value objects (Polarity, SpeculationIndex, Confidence, Alignment, SourceKind, PostSignal)

**Files:**
- Modify: `src/domain/values/polarity.rs`, `speculation.rs`, `source_kind.rs`, `post_signal.rs`

**Interfaces:**
- Produces:
  - `Polarity` — `new(f64) -> Self` (clamps `[-1,1]`), `value(self) -> f64`. `Copy`, `Serialize` (transparent).
  - `SpeculationIndex` — `new(f64) -> Self` (clamps `[0,1]`), `value(self) -> f64`. `Copy`, `Serialize` (transparent).
  - `Confidence` — enum `{ Low, Medium, High }`; `from_sample(n: usize, low: usize, high: usize) -> Self`. `Serialize` (lowercase).
  - `Alignment` — enum `{ ConfirmingBullish, ConfirmingBearish, Diverging, Quiet }`. `Serialize` (snake_case).
  - `SourceKind` — enum `{ Reddit, X, Bluesky }`; `as_str(self) -> &'static str`. `Copy, Ord, Serialize` (lowercase).
  - `PostSignal` — struct `{ polarity: Polarity, speculative: bool }`. `Copy`.

- [ ] **Step 1: Write the failing tests**

`src/domain/values/polarity.rs`
```rust
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Polarity(f64);

impl Polarity {
    pub fn new(value: f64) -> Self {
        Polarity(value.clamp(-1.0, 1.0))
    }

    pub fn value(self) -> f64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_out_of_range() {
        assert_eq!(Polarity::new(5.0).value(), 1.0);
        assert_eq!(Polarity::new(-5.0).value(), -1.0);
        assert_eq!(Polarity::new(0.3).value(), 0.3);
    }
}
```

`src/domain/values/speculation.rs`
```rust
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(transparent)]
pub struct SpeculationIndex(f64);

impl SpeculationIndex {
    pub fn new(value: f64) -> Self {
        SpeculationIndex(value.clamp(0.0, 1.0))
    }

    pub fn value(self) -> f64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    Low,
    Medium,
    High,
}

impl Confidence {
    /// `n < low` -> Low, `low <= n < high` -> Medium, `n >= high` -> High.
    pub fn from_sample(n: usize, low: usize, high: usize) -> Self {
        if n < low {
            Confidence::Low
        } else if n < high {
            Confidence::Medium
        } else {
            Confidence::High
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Alignment {
    ConfirmingBullish,
    ConfirmingBearish,
    Diverging,
    Quiet,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speculation_index_clamps() {
        assert_eq!(SpeculationIndex::new(1.5).value(), 1.0);
        assert_eq!(SpeculationIndex::new(-0.2).value(), 0.0);
    }

    #[test]
    fn confidence_buckets() {
        assert_eq!(Confidence::from_sample(5, 10, 50), Confidence::Low);
        assert_eq!(Confidence::from_sample(10, 10, 50), Confidence::Medium);
        assert_eq!(Confidence::from_sample(49, 10, 50), Confidence::Medium);
        assert_eq!(Confidence::from_sample(50, 10, 50), Confidence::High);
    }
}
```

`src/domain/values/source_kind.rs`
```rust
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    Reddit,
    X,
    Bluesky,
}

impl SourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SourceKind::Reddit => "reddit",
            SourceKind::X => "x",
            SourceKind::Bluesky => "bluesky",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_str_matches() {
        assert_eq!(SourceKind::Reddit.as_str(), "reddit");
        assert_eq!(SourceKind::X.as_str(), "x");
        assert_eq!(SourceKind::Bluesky.as_str(), "bluesky");
    }

    #[test]
    fn serializes_lowercase() {
        let json = serde_json::to_string(&SourceKind::Bluesky).unwrap();
        assert_eq!(json, "\"bluesky\"");
    }
}
```

`src/domain/values/post_signal.rs`
```rust
use crate::domain::values::polarity::Polarity;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PostSignal {
    pub polarity: Polarity,
    pub speculative: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holds_polarity_and_flag() {
        let s = PostSignal { polarity: Polarity::new(0.5), speculative: true };
        assert_eq!(s.polarity.value(), 0.5);
        assert!(s.speculative);
    }
}
```

- [ ] **Step 2: Run tests to verify they pass** (these files contain both impl and tests)

Run: `cargo test domain::values`
Expected: PASS (all value-object tests).

- [ ] **Step 3: Lint & format**

Run: `cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add src/domain/values
git commit -m "feat(domain): add value objects (polarity, speculation, source kind, signal)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 4: Ticker entity

**Files:**
- Modify: `src/domain/entities/ticker.rs`

**Interfaces:**
- Consumes: `DomainError`.
- Produces: `Ticker` — `parse(&str) -> Result<Ticker, DomainError>` (uppercases, validates 1–5 ASCII letters + optional single `.CLASS` letter), `as_str(&self) -> &str`. `Clone, Eq, Serialize` (transparent).

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_symbols() {
        assert_eq!(Ticker::parse("aapl").unwrap().as_str(), "AAPL");
        assert_eq!(Ticker::parse("BRK.B").unwrap().as_str(), "BRK.B");
    }

    #[test]
    fn rejects_invalid_symbols() {
        for bad in ["", "   ", "TOOLONG", "A1", "AB.CD", "AAPL.", "$AAPL"] {
            assert!(Ticker::parse(bad).is_err(), "expected {bad:?} to be rejected");
        }
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test domain::entities::ticker`
Expected: FAIL — `Ticker` not found.

- [ ] **Step 3: Write the implementation** — prepend above the tests

```rust
use serde::Serialize;

use crate::domain::error::DomainError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Ticker(String);

impl Ticker {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let symbol = raw.trim().to_uppercase();
        if symbol.is_empty() {
            return Err(DomainError::InvalidTicker("empty".into()));
        }

        let (base, class) = match symbol.split_once('.') {
            Some((b, c)) => (b, Some(c)),
            None => (symbol.as_str(), None),
        };

        let base_ok =
            (1..=5).contains(&base.len()) && base.chars().all(|c| c.is_ascii_uppercase());
        let class_ok = match class {
            None => true,
            Some(c) => c.len() == 1 && c.chars().all(|c| c.is_ascii_uppercase()),
        };

        if base_ok && class_ok {
            Ok(Ticker(symbol))
        } else {
            Err(DomainError::InvalidTicker(raw.to_string()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test domain::entities::ticker && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: PASS, clean.

- [ ] **Step 5: Commit**

```bash
git add src/domain/entities/ticker.rs
git commit -m "feat(domain): add Ticker with validation

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 5: SocialPost + PostText

**Files:**
- Modify: `src/domain/entities/social_post.rs`

**Interfaces:**
- Consumes: `DomainError`, `SourceKind`.
- Produces:
  - `PostText` — `parse(&str) -> Result<PostText, DomainError>` (trims, rejects empty / `> 10_000` chars), `as_str(&self) -> &str`. `Clone, Eq, Serialize` (transparent).
  - `SocialPost` — public struct `{ id: String, source: SourceKind, author: String, text: PostText, created_at: DateTime<Utc>, engagement: u32 }`. `Clone, Serialize`.

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn post_text_trims_and_rejects_empty() {
        assert_eq!(PostText::parse("  hello  ").unwrap().as_str(), "hello");
        assert!(PostText::parse("   ").is_err());
        assert!(PostText::parse(&"x".repeat(10_001)).is_err());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test domain::entities::social_post`
Expected: FAIL — `PostText` not found.

- [ ] **Step 3: Write the implementation** — prepend above the tests

```rust
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::domain::error::DomainError;
use crate::domain::values::source_kind::SourceKind;

const MAX_POST_LEN: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct PostText(String);

impl PostText {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(DomainError::InvalidPostText("empty".into()));
        }
        if trimmed.len() > MAX_POST_LEN {
            return Err(DomainError::InvalidPostText("exceeds max length".into()));
        }
        Ok(PostText(trimmed.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SocialPost {
    pub id: String,
    pub source: SourceKind,
    pub author: String,
    pub text: PostText,
    pub created_at: DateTime<Utc>,
    pub engagement: u32,
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test domain::entities::social_post && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: PASS, clean.

- [ ] **Step 5: Commit**

```bash
git add src/domain/entities/social_post.rs
git commit -m "feat(domain): add SocialPost and PostText

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 6: MarketSnapshot + report types

**Files:**
- Modify: `src/domain/entities/market_snapshot.rs`, `src/domain/entities/speculation_report.rs`

**Interfaces:**
- Consumes: `Ticker`, `Polarity`, `SpeculationIndex`, `Confidence`, `Alignment`, `SourceKind`.
- Produces:
  - `MarketSnapshot` — public struct `{ ticker: Ticker, as_of: DateTime<Utc>, last_price: f64, previous_close: f64, volume: u64, avg_volume: u64, realized_vol: Option<f64>, put_call_ratio: Option<f64>, iv_rank: Option<f64> }`. `Clone, Serialize`.
  - `SocialSummary`, `MarketSummary`, `FusionSignals`, `SpeculationReport` (fields exactly as below). All `Clone, Serialize`.

- [ ] **Step 1: Write `src/domain/entities/market_snapshot.rs`**

```rust
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::domain::entities::ticker::Ticker;

#[derive(Debug, Clone, Serialize)]
pub struct MarketSnapshot {
    pub ticker: Ticker,
    pub as_of: DateTime<Utc>,
    pub last_price: f64,
    pub previous_close: f64,
    pub volume: u64,
    pub avg_volume: u64,
    pub realized_vol: Option<f64>,
    pub put_call_ratio: Option<f64>,
    pub iv_rank: Option<f64>,
}
```

- [ ] **Step 2: Write `src/domain/entities/speculation_report.rs`**

```rust
use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::domain::entities::ticker::Ticker;
use crate::domain::values::polarity::Polarity;
use crate::domain::values::source_kind::SourceKind;
use crate::domain::values::speculation::{Alignment, Confidence, SpeculationIndex};

#[derive(Debug, Clone, Serialize)]
pub struct SocialSummary {
    pub total_mentions: usize,
    pub mentions_by_source: BTreeMap<SourceKind, usize>,
    pub net_sentiment: Polarity,
    pub bullish: usize,
    pub bearish: usize,
    pub neutral: usize,
    pub bull_bear_ratio: Option<f64>,
    pub speculation_index: SpeculationIndex,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarketSummary {
    pub last_price: f64,
    pub pct_change: f64,
    pub rvol: f64,
    pub realized_vol: Option<f64>,
    pub put_call_ratio: Option<f64>,
    pub iv_rank: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FusionSignals {
    pub alignment: Alignment,
    pub crowding: f64,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpeculationReport {
    pub ticker: Ticker,
    pub generated_at: DateTime<Utc>,
    pub social: SocialSummary,
    pub market: Option<MarketSummary>,
    pub fusion: FusionSignals,
    pub social_confidence: Confidence,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn serializes_with_enum_source_keys() {
        let mut by_source = BTreeMap::new();
        by_source.insert(SourceKind::Reddit, 2);
        let report = SpeculationReport {
            ticker: Ticker::parse("AAPL").unwrap(),
            generated_at: Utc.with_ymd_and_hms(2026, 6, 24, 0, 0, 0).unwrap(),
            social: SocialSummary {
                total_mentions: 2,
                mentions_by_source: by_source,
                net_sentiment: Polarity::new(0.4),
                bullish: 2,
                bearish: 0,
                neutral: 0,
                bull_bear_ratio: None,
                speculation_index: SpeculationIndex::new(0.5),
            },
            market: None,
            fusion: FusionSignals {
                alignment: Alignment::Quiet,
                crowding: 0.25,
                notes: vec![],
            },
            social_confidence: Confidence::Low,
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"reddit\":2"));
        assert!(json.contains("\"speculation_index\":0.5"));
        assert!(json.contains("\"alignment\":\"quiet\""));
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test domain::entities && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: PASS, clean. (Confirms enum-keyed `BTreeMap` serializes to `{"reddit":2}`.)

- [ ] **Step 4: Commit**

```bash
git add src/domain/entities/market_snapshot.rs src/domain/entities/speculation_report.rs
git commit -m "feat(domain): add MarketSnapshot and SpeculationReport types

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 7: Ports (traits)

**Files:**
- Modify: `src/domain/ports/social_data_source.rs`, `market_data_source.rs`, `post_analyzer.rs`

**Interfaces:**
- Consumes: `SocialPost`, `MarketSnapshot`, `PostSignal`, `Ticker`, `SourceKind`, `DomainError`.
- Produces (all `: Send + Sync`, `#[async_trait]`):
  - `SocialDataSource` — `fn kind(&self) -> SourceKind; async fn fetch(&self, ticker: &Ticker, limit: usize) -> Result<Vec<SocialPost>, DomainError>;`
  - `MarketDataSource` — `fn name(&self) -> &'static str; async fn snapshot(&self, ticker: &Ticker) -> Result<MarketSnapshot, DomainError>;`
  - `PostAnalyzer` — `async fn analyze(&self, posts: &[SocialPost]) -> Result<Vec<PostSignal>, DomainError>;`

- [ ] **Step 1: Write `src/domain/ports/social_data_source.rs`**

```rust
use async_trait::async_trait;

use crate::domain::entities::social_post::SocialPost;
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::values::source_kind::SourceKind;

#[async_trait]
pub trait SocialDataSource: Send + Sync {
    fn kind(&self) -> SourceKind;
    async fn fetch(&self, ticker: &Ticker, limit: usize) -> Result<Vec<SocialPost>, DomainError>;
}
```

- [ ] **Step 2: Write `src/domain/ports/market_data_source.rs`**

```rust
use async_trait::async_trait;

use crate::domain::entities::market_snapshot::MarketSnapshot;
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;

#[async_trait]
pub trait MarketDataSource: Send + Sync {
    fn name(&self) -> &'static str;
    async fn snapshot(&self, ticker: &Ticker) -> Result<MarketSnapshot, DomainError>;
}
```

- [ ] **Step 3: Write `src/domain/ports/post_analyzer.rs` (with a trait-usability test)**

```rust
use async_trait::async_trait;

use crate::domain::entities::social_post::SocialPost;
use crate::domain::error::DomainError;
use crate::domain::values::post_signal::PostSignal;

#[async_trait]
pub trait PostAnalyzer: Send + Sync {
    /// One `PostSignal` per input post, aligned to input order (`len == posts.len()`).
    async fn analyze(&self, posts: &[SocialPost]) -> Result<Vec<PostSignal>, DomainError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::values::polarity::Polarity;

    struct ConstAnalyzer;

    #[async_trait]
    impl PostAnalyzer for ConstAnalyzer {
        async fn analyze(&self, posts: &[SocialPost]) -> Result<Vec<PostSignal>, DomainError> {
            Ok(posts
                .iter()
                .map(|_| PostSignal { polarity: Polarity::new(0.0), speculative: false })
                .collect())
        }
    }

    #[tokio::test]
    async fn trait_is_object_safe_and_callable() {
        let analyzer: Box<dyn PostAnalyzer> = Box::new(ConstAnalyzer);
        let out = analyzer.analyze(&[]).await.unwrap();
        assert!(out.is_empty());
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test domain::ports && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: PASS, clean.

- [ ] **Step 5: Commit**

```bash
git add src/domain/ports
git commit -m "feat(domain): add SocialDataSource, MarketDataSource, PostAnalyzer ports

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 8: EngineConfig

**Files:**
- Modify: `src/domain/engine/config.rs`

**Interfaces:**
- Produces: `EngineConfig` (public fields below) with `Default`:
  `bull_bear_threshold: f64`, `net_sentiment_threshold: f64`, `price_move_threshold: f64`, `crowding_weight_spec: f64`, `crowding_weight_rvol: f64`, `crowding_weight_iv: f64`, `rvol_cap: f64`, `min_sample: usize`, `confidence_low: usize`, `confidence_high: usize`.

- [ ] **Step 1: Write the file (impl + test)**

```rust
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// τ — per-post bull/bear classification threshold.
    pub bull_bear_threshold: f64,
    /// σ — aggregate net-sentiment threshold for alignment.
    pub net_sentiment_threshold: f64,
    /// δ — minimum |pct_change| (percent) to count as a meaningful price move.
    pub price_move_threshold: f64,
    pub crowding_weight_spec: f64,
    pub crowding_weight_rvol: f64,
    pub crowding_weight_iv: f64,
    pub rvol_cap: f64,
    pub min_sample: usize,
    pub confidence_low: usize,
    pub confidence_high: usize,
}

impl Default for EngineConfig {
    fn default() -> Self {
        EngineConfig {
            bull_bear_threshold: 0.2,
            net_sentiment_threshold: 0.05,
            price_move_threshold: 1.0,
            crowding_weight_spec: 0.5,
            crowding_weight_rvol: 0.3,
            crowding_weight_iv: 0.2,
            rvol_cap: 3.0,
            min_sample: 10,
            confidence_low: 10,
            confidence_high: 50,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_spec() {
        let c = EngineConfig::default();
        assert_eq!(c.bull_bear_threshold, 0.2);
        assert_eq!(c.net_sentiment_threshold, 0.05);
        assert_eq!(c.min_sample, 10);
        assert_eq!((c.confidence_low, c.confidence_high), (10, 50));
    }
}
```

- [ ] **Step 2: Run test & lint**

Run: `cargo test domain::engine::config && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: PASS, clean.

- [ ] **Step 3: Commit**

```bash
git add src/domain/engine/config.rs
git commit -m "feat(domain): add EngineConfig with spec defaults

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 9: SpeculationEngine (the pure core)

**Files:**
- Modify: `src/domain/engine/speculation_engine.rs`

**Interfaces:**
- Consumes: `Ticker`, `SocialPost`, `PostSignal`, `MarketSnapshot`, `EngineConfig`, report types, `DomainError`, `SourceKind`, `Polarity`, `SpeculationIndex`, `Confidence`, `Alignment`.
- Produces: `SpeculationEngine::aggregate(ticker: &Ticker, posts: &[SocialPost], signals: &[PostSignal], market: Option<&MarketSnapshot>, now: DateTime<Utc>, cfg: &EngineConfig) -> Result<SpeculationReport, DomainError>`.

- [ ] **Step 1: Write the full test suite first** (append after the impl in Step 3; here is the test module to add)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ticker() -> Ticker {
        Ticker::parse("AAPL").unwrap()
    }
    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 24, 0, 0, 0).unwrap()
    }
    fn post(source: SourceKind) -> SocialPost {
        SocialPost {
            id: "x".into(),
            source,
            author: "a".into(),
            text: PostText::parse("placeholder").unwrap(),
            created_at: now(),
            engagement: 0,
        }
    }
    fn sig(polarity: f64, speculative: bool) -> PostSignal {
        PostSignal { polarity: Polarity::new(polarity), speculative }
    }
    fn snapshot(last: f64, prev: f64, vol: u64, avg: u64, iv: Option<f64>) -> MarketSnapshot {
        MarketSnapshot {
            ticker: ticker(),
            as_of: now(),
            last_price: last,
            previous_close: prev,
            volume: vol,
            avg_volume: avg,
            realized_vol: None,
            put_call_ratio: None,
            iv_rank: iv,
        }
    }
    /// 12 posts: 9 bullish (+0.8), 3 neutral (0.0) — net ≈ 0.6, all reddit.
    fn bullish_batch() -> (Vec<SocialPost>, Vec<PostSignal>) {
        let posts: Vec<_> = (0..12).map(|_| post(SourceKind::Reddit)).collect();
        let mut signals = vec![sig(0.8, true); 9];
        signals.extend(vec![sig(0.0, false); 3]);
        (posts, signals)
    }

    #[test]
    fn confirming_bullish_when_sentiment_and_price_agree() {
        let (posts, signals) = bullish_batch();
        let m = snapshot(110.0, 100.0, 1, 1, Some(0.5)); // +10%
        let report =
            SpeculationEngine::aggregate(&ticker(), &posts, &signals, Some(&m), now(), &EngineConfig::default())
                .unwrap();
        assert_eq!(report.fusion.alignment, Alignment::ConfirmingBullish);
        assert_eq!(report.social.bullish, 9);
        assert_eq!(report.social_confidence, Confidence::Medium); // 12 mentions
        assert!(report.market.is_some());
    }

    #[test]
    fn diverging_when_sentiment_up_but_price_down() {
        let (posts, signals) = bullish_batch();
        let m = snapshot(90.0, 100.0, 1, 1, None); // -10%
        let report =
            SpeculationEngine::aggregate(&ticker(), &posts, &signals, Some(&m), now(), &EngineConfig::default())
                .unwrap();
        assert_eq!(report.fusion.alignment, Alignment::Diverging);
    }

    #[test]
    fn empty_input_is_quiet_and_zeroed() {
        let report =
            SpeculationEngine::aggregate(&ticker(), &[], &[], None, now(), &EngineConfig::default()).unwrap();
        assert_eq!(report.social.total_mentions, 0);
        assert_eq!(report.social.net_sentiment.value(), 0.0);
        assert_eq!(report.social.speculation_index.value(), 0.0);
        assert_eq!(report.fusion.alignment, Alignment::Quiet);
        assert_eq!(report.fusion.crowding, 0.0);
        assert_eq!(report.social_confidence, Confidence::Low);
    }

    #[test]
    fn no_market_forces_quiet_alignment() {
        let (posts, signals) = bullish_batch();
        let report =
            SpeculationEngine::aggregate(&ticker(), &posts, &signals, None, now(), &EngineConfig::default())
                .unwrap();
        assert!(report.market.is_none());
        assert_eq!(report.fusion.alignment, Alignment::Quiet);
        assert!(report.fusion.notes.iter().any(|n| n.contains("social-only")));
    }

    #[test]
    fn length_mismatch_errors() {
        let posts = vec![post(SourceKind::Reddit), post(SourceKind::Reddit)];
        let signals = vec![sig(0.5, false)];
        let err = SpeculationEngine::aggregate(&ticker(), &posts, &signals, None, now(), &EngineConfig::default())
            .unwrap_err();
        assert!(matches!(err, DomainError::AnalyzerMismatch { expected: 2, got: 1 }));
    }

    #[test]
    fn bull_bear_ratio_is_none_without_bears() {
        let posts = vec![post(SourceKind::Reddit)];
        let signals = vec![sig(0.9, false)];
        let report =
            SpeculationEngine::aggregate(&ticker(), &posts, &signals, None, now(), &EngineConfig::default())
                .unwrap();
        assert_eq!(report.social.bull_bear_ratio, None);
    }

    #[test]
    fn rvol_guarded_when_avg_volume_zero() {
        let posts = vec![post(SourceKind::Reddit)];
        let signals = vec![sig(0.0, false)];
        let m = snapshot(100.0, 100.0, 10, 0, None);
        let report =
            SpeculationEngine::aggregate(&ticker(), &posts, &signals, Some(&m), now(), &EngineConfig::default())
                .unwrap();
        assert_eq!(report.market.unwrap().rvol, 0.0);
        assert!(report.fusion.notes.iter().any(|n| n.contains("avg_volume")));
    }

    #[test]
    fn crowding_renormalizes_without_market() {
        // social-only: every post speculative -> speculation_index 1.0 -> crowding == 1.0
        let posts: Vec<_> = (0..3).map(|_| post(SourceKind::Reddit)).collect();
        let signals = vec![sig(0.0, true); 3];
        let report =
            SpeculationEngine::aggregate(&ticker(), &posts, &signals, None, now(), &EngineConfig::default())
                .unwrap();
        assert_eq!(report.fusion.crowding, 1.0);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test domain::engine::speculation_engine`
Expected: FAIL — `SpeculationEngine` not found.

- [ ] **Step 3: Write the implementation** — prepend above the test module

```rust
use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

use crate::domain::engine::config::EngineConfig;
use crate::domain::entities::market_snapshot::MarketSnapshot;
use crate::domain::entities::social_post::SocialPost;
use crate::domain::entities::speculation_report::{
    FusionSignals, MarketSummary, SocialSummary, SpeculationReport,
};
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::values::polarity::Polarity;
use crate::domain::values::post_signal::PostSignal;
use crate::domain::values::source_kind::SourceKind;
use crate::domain::values::speculation::{Alignment, Confidence, SpeculationIndex};

pub struct SpeculationEngine;

impl SpeculationEngine {
    pub fn aggregate(
        ticker: &Ticker,
        posts: &[SocialPost],
        signals: &[PostSignal],
        market: Option<&MarketSnapshot>,
        now: DateTime<Utc>,
        cfg: &EngineConfig,
    ) -> Result<SpeculationReport, DomainError> {
        if signals.len() != posts.len() {
            return Err(DomainError::AnalyzerMismatch {
                expected: posts.len(),
                got: signals.len(),
            });
        }

        let mut notes: Vec<String> = Vec::new();
        let social = Self::social_summary(posts, signals, cfg);
        let market_summary = market.map(|m| Self::market_summary(m, &mut notes));
        let crowding = Self::crowding(&social, market_summary.as_ref(), cfg);
        let alignment = Self::alignment(&social, market_summary.as_ref(), cfg, &mut notes);
        let social_confidence =
            Confidence::from_sample(social.total_mentions, cfg.confidence_low, cfg.confidence_high);

        Ok(SpeculationReport {
            ticker: ticker.clone(),
            generated_at: now,
            social,
            market: market_summary,
            fusion: FusionSignals { alignment, crowding, notes },
            social_confidence,
        })
    }

    fn social_summary(posts: &[SocialPost], signals: &[PostSignal], cfg: &EngineConfig) -> SocialSummary {
        let total = posts.len();
        let mut by_source: BTreeMap<SourceKind, usize> = BTreeMap::new();
        for p in posts {
            *by_source.entry(p.source).or_insert(0) += 1;
        }

        let (mut bullish, mut bearish, mut neutral, mut spec_count) = (0usize, 0usize, 0usize, 0usize);
        let mut polarity_sum = 0.0f64;
        for s in signals {
            let v = s.polarity.value();
            polarity_sum += v;
            if v > cfg.bull_bear_threshold {
                bullish += 1;
            } else if v < -cfg.bull_bear_threshold {
                bearish += 1;
            } else {
                neutral += 1;
            }
            if s.speculative {
                spec_count += 1;
            }
        }

        let net = if total == 0 { 0.0 } else { polarity_sum / total as f64 };
        let spec_index = if total == 0 { 0.0 } else { spec_count as f64 / total as f64 };
        let bull_bear_ratio = if bearish == 0 {
            None
        } else {
            Some(bullish as f64 / bearish as f64)
        };

        SocialSummary {
            total_mentions: total,
            mentions_by_source: by_source,
            net_sentiment: Polarity::new(net),
            bullish,
            bearish,
            neutral,
            bull_bear_ratio,
            speculation_index: SpeculationIndex::new(spec_index),
        }
    }

    fn market_summary(m: &MarketSnapshot, notes: &mut Vec<String>) -> MarketSummary {
        let pct_change = if m.previous_close == 0.0 {
            notes.push("previous_close is 0; pct_change set to 0".to_string());
            0.0
        } else {
            (m.last_price - m.previous_close) / m.previous_close * 100.0
        };
        let rvol = if m.avg_volume == 0 {
            notes.push("avg_volume is 0; rvol set to 0".to_string());
            0.0
        } else {
            m.volume as f64 / m.avg_volume as f64
        };
        MarketSummary {
            last_price: m.last_price,
            pct_change,
            rvol,
            realized_vol: m.realized_vol,
            put_call_ratio: m.put_call_ratio,
            iv_rank: m.iv_rank,
        }
    }

    /// Weighted blend of available components, renormalized over present weights.
    fn crowding(social: &SocialSummary, market: Option<&MarketSummary>, cfg: &EngineConfig) -> f64 {
        let mut weighted = 0.0f64;
        let mut weight_sum = 0.0f64;

        if social.total_mentions > 0 {
            weighted += cfg.crowding_weight_spec * social.speculation_index.value();
            weight_sum += cfg.crowding_weight_spec;
        }
        if let Some(m) = market {
            let rvol_norm = (m.rvol / cfg.rvol_cap).clamp(0.0, 1.0);
            weighted += cfg.crowding_weight_rvol * rvol_norm;
            weight_sum += cfg.crowding_weight_rvol;
            if let Some(iv) = m.iv_rank {
                weighted += cfg.crowding_weight_iv * iv.clamp(0.0, 1.0);
                weight_sum += cfg.crowding_weight_iv;
            }
        }

        if weight_sum == 0.0 {
            0.0
        } else {
            (weighted / weight_sum).clamp(0.0, 1.0)
        }
    }

    fn alignment(
        social: &SocialSummary,
        market: Option<&MarketSummary>,
        cfg: &EngineConfig,
        notes: &mut Vec<String>,
    ) -> Alignment {
        let market = match market {
            None => {
                notes.push("social-only, no price reference".to_string());
                return Alignment::Quiet;
            }
            Some(m) => m,
        };
        if social.total_mentions < cfg.min_sample {
            return Alignment::Quiet;
        }

        let s = social.net_sentiment.value();
        let p = market.pct_change;
        let sentiment_meaningful = s.abs() >= cfg.net_sentiment_threshold;
        let price_meaningful = p.abs() >= cfg.price_move_threshold;
        if !sentiment_meaningful || !price_meaningful {
            return Alignment::Quiet;
        }

        match (s > 0.0, p > 0.0) {
            (true, true) => Alignment::ConfirmingBullish,
            (false, false) => Alignment::ConfirmingBearish,
            _ => Alignment::Diverging,
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test domain::engine::speculation_engine && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: PASS (all 8 engine tests), clean.

- [ ] **Step 5: Commit**

```bash
git add src/domain/engine/speculation_engine.rs
git commit -m "feat(domain): add pure SpeculationEngine with fusion + edge handling

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 10: LexiconAnalyzer adapter

**Files:**
- Modify: `src/adapters/analyzer/lexicon.rs`

**Interfaces:**
- Consumes: `PostAnalyzer`, `SocialPost`, `PostSignal`, `Polarity`, `DomainError`.
- Produces: `LexiconAnalyzer` — `new() -> Self`, implements `PostAnalyzer` and `Default`. Offline scoring: polarity from bull/bear word hits, `speculative` from options-jargon hits.

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::values::source_kind::SourceKind;
    use chrono::Utc;

    fn post(text: &str) -> SocialPost {
        SocialPost {
            id: "1".into(),
            source: SourceKind::Reddit,
            author: "a".into(),
            text: crate::domain::entities::social_post::PostText::parse(text).unwrap(),
            created_at: Utc::now(),
            engagement: 0,
        }
    }

    #[tokio::test]
    async fn scores_sentiment_and_speculation() {
        let analyzer = LexiconAnalyzer::new();
        let posts = vec![
            post("to the moon, buying calls"), // bullish + speculative
            post("this will dump, buying puts"), // bearish + speculative
            post("the company released a quarterly report"), // neutral, no jargon
        ];
        let signals = analyzer.analyze(&posts).await.unwrap();
        assert_eq!(signals.len(), 3);
        assert!(signals[0].polarity.value() > 0.0 && signals[0].speculative);
        assert!(signals[1].polarity.value() < 0.0 && signals[1].speculative);
        assert_eq!(signals[2].polarity.value(), 0.0);
        assert!(!signals[2].speculative);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test adapters::analyzer::lexicon`
Expected: FAIL — `LexiconAnalyzer` not found.

- [ ] **Step 3: Write the implementation** — prepend above the tests

```rust
use async_trait::async_trait;

use crate::domain::entities::social_post::SocialPost;
use crate::domain::error::DomainError;
use crate::domain::ports::post_analyzer::PostAnalyzer;
use crate::domain::values::polarity::Polarity;
use crate::domain::values::post_signal::PostSignal;

const BULL: &[&str] = &[
    "moon", "calls", "long", "buy", "bullish", "squeeze", "breakout", "rocket", "pump", "rip",
    "green", "up", "rally", "bull",
];
const BEAR: &[&str] = &[
    "puts", "short", "sell", "bearish", "dump", "crash", "drilling", "bagholder", "rug", "red",
    "down", "tank", "bear",
];
const JARGON: &[&str] = &[
    "calls", "puts", "0dte", "yolo", "leaps", "theta", "gamma", "squeeze", "otm", "itm", "strike",
    "iv", "delta", "vega", "contracts",
];

pub struct LexiconAnalyzer;

impl LexiconAnalyzer {
    pub fn new() -> Self {
        LexiconAnalyzer
    }

    fn score(text: &str) -> PostSignal {
        let lower = text.to_lowercase();
        let tokens: Vec<&str> = lower
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|t| !t.is_empty())
            .collect();

        let bull_hits = tokens.iter().filter(|t| BULL.contains(t)).count() as f64;
        let bear_hits = tokens.iter().filter(|t| BEAR.contains(t)).count() as f64;
        let polarity = if bull_hits + bear_hits == 0.0 {
            0.0
        } else {
            (bull_hits - bear_hits) / (bull_hits + bear_hits)
        };
        let speculative = tokens.iter().any(|t| JARGON.contains(t));

        PostSignal { polarity: Polarity::new(polarity), speculative }
    }
}

impl Default for LexiconAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PostAnalyzer for LexiconAnalyzer {
    async fn analyze(&self, posts: &[SocialPost]) -> Result<Vec<PostSignal>, DomainError> {
        Ok(posts.iter().map(|p| Self::score(p.text.as_str())).collect())
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test adapters::analyzer::lexicon && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: PASS, clean.

- [ ] **Step 5: Commit**

```bash
git add src/adapters/analyzer/lexicon.rs
git commit -m "feat(adapters): add offline LexiconAnalyzer (PostAnalyzer impl)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 11: Mock social sources

**Files:**
- Modify: `src/adapters/sources/mock_reddit.rs`, `mock_x.rs`, `mock_bluesky.rs`

**Interfaces:**
- Consumes: `SocialDataSource`, `SocialPost`, `PostText`, `Ticker`, `SourceKind`, `DomainError`.
- Produces: `MockRedditSource`, `MockXSource`, `MockBlueskySource` — zero-sized structs implementing `SocialDataSource` with deterministic fixtures and fixed timestamps, honoring `limit`.

- [ ] **Step 1: Write `src/adapters/sources/mock_reddit.rs`**

```rust
use async_trait::async_trait;
use chrono::{TimeZone, Utc};

use crate::domain::entities::social_post::{PostText, SocialPost};
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::ports::social_data_source::SocialDataSource;
use crate::domain::values::source_kind::SourceKind;

pub struct MockRedditSource;

#[async_trait]
impl SocialDataSource for MockRedditSource {
    fn kind(&self) -> SourceKind {
        SourceKind::Reddit
    }

    async fn fetch(&self, ticker: &Ticker, limit: usize) -> Result<Vec<SocialPost>, DomainError> {
        let sym = ticker.as_str();
        let fixtures = [
            ("reddit-1", "dudebro", format!("{sym} to the moon, loading calls all day"), 420u32),
            ("reddit-2", "valuepicker", format!("{sym} earnings look strong, going long here"), 88),
            ("reddit-3", "chartwatcher", format!("{sym} breakout confirmed, rocket time"), 51),
            ("reddit-4", "shortking", format!("{sym} is going to dump, buying puts"), 31),
        ];
        Ok(fixtures
            .iter()
            .take(limit)
            .map(|(id, author, text, eng)| SocialPost {
                id: (*id).to_string(),
                source: SourceKind::Reddit,
                author: (*author).to_string(),
                text: PostText::parse(text).expect("fixture text is valid"),
                created_at: Utc.with_ymd_and_hms(2026, 6, 24, 14, 0, 0).unwrap(),
                engagement: *eng,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn returns_deterministic_posts_and_honors_limit() {
        let src = MockRedditSource;
        let t = Ticker::parse("AAPL").unwrap();
        let all = src.fetch(&t, 50).await.unwrap();
        assert_eq!(all.len(), 4);
        assert!(all[0].text.as_str().contains("AAPL"));
        assert_eq!(src.fetch(&t, 1).await.unwrap().len(), 1);
        assert_eq!(src.kind(), SourceKind::Reddit);
    }
}
```

- [ ] **Step 2: Write `src/adapters/sources/mock_x.rs`** (same shape, X fixtures)

```rust
use async_trait::async_trait;
use chrono::{TimeZone, Utc};

use crate::domain::entities::social_post::{PostText, SocialPost};
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::ports::social_data_source::SocialDataSource;
use crate::domain::values::source_kind::SourceKind;

pub struct MockXSource;

#[async_trait]
impl SocialDataSource for MockXSource {
    fn kind(&self) -> SourceKind {
        SourceKind::X
    }

    async fn fetch(&self, ticker: &Ticker, limit: usize) -> Result<Vec<SocialPost>, DomainError> {
        let sym = ticker.as_str();
        let fixtures = [
            ("x-1", "quanttrader", format!("${sym} squeeze incoming, buying calls"), 1200u32),
            ("x-2", "macroowl", format!("watching ${sym} but staying cautious"), 64),
            ("x-3", "trendrider", format!("${sym} rally looks strong"), 240),
        ];
        Ok(fixtures
            .iter()
            .take(limit)
            .map(|(id, author, text, eng)| SocialPost {
                id: (*id).to_string(),
                source: SourceKind::X,
                author: (*author).to_string(),
                text: PostText::parse(text).expect("fixture text is valid"),
                created_at: Utc.with_ymd_and_hms(2026, 6, 24, 15, 0, 0).unwrap(),
                engagement: *eng,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn returns_posts() {
        let posts = MockXSource.fetch(&Ticker::parse("AAPL").unwrap(), 50).await.unwrap();
        assert_eq!(posts.len(), 3);
        assert_eq!(MockXSource.kind(), SourceKind::X);
    }
}
```

- [ ] **Step 3: Write `src/adapters/sources/mock_bluesky.rs`** (same shape, Bluesky fixtures)

```rust
use async_trait::async_trait;
use chrono::{TimeZone, Utc};

use crate::domain::entities::social_post::{PostText, SocialPost};
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::ports::social_data_source::SocialDataSource;
use crate::domain::values::source_kind::SourceKind;

pub struct MockBlueskySource;

#[async_trait]
impl SocialDataSource for MockBlueskySource {
    fn kind(&self) -> SourceKind {
        SourceKind::Bluesky
    }

    async fn fetch(&self, ticker: &Ticker, limit: usize) -> Result<Vec<SocialPost>, DomainError> {
        let sym = ticker.as_str();
        let fixtures = [
            ("bsky-1", "indexfan", format!("{sym} looking bullish into the print"), 22u32),
            ("bsky-2", "skeptic", format!("not sold on {sym}, might sell my shares"), 9),
            ("bsky-3", "daytripper", format!("{sym} green day, up big"), 14),
        ];
        Ok(fixtures
            .iter()
            .take(limit)
            .map(|(id, author, text, eng)| SocialPost {
                id: (*id).to_string(),
                source: SourceKind::Bluesky,
                author: (*author).to_string(),
                text: PostText::parse(text).expect("fixture text is valid"),
                created_at: Utc.with_ymd_and_hms(2026, 6, 24, 16, 0, 0).unwrap(),
                engagement: *eng,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn returns_posts() {
        let posts = MockBlueskySource.fetch(&Ticker::parse("AAPL").unwrap(), 50).await.unwrap();
        assert_eq!(posts.len(), 3);
        assert_eq!(MockBlueskySource.kind(), SourceKind::Bluesky);
    }
}
```

- [ ] **Step 4: Run tests & lint**

Run: `cargo test adapters::sources && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: PASS, clean.

- [ ] **Step 5: Commit**

```bash
git add src/adapters/sources
git commit -m "feat(adapters): add mock Reddit/X/Bluesky social sources

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 12: Mock market source

**Files:**
- Modify: `src/adapters/market/mock_market.rs`

**Interfaces:**
- Consumes: `MarketDataSource`, `MarketSnapshot`, `Ticker`, `DomainError`.
- Produces: `MockMarketSource` — implements `MarketDataSource`, returns a deterministic up-day snapshot (`+~4%`, RVOL `~1.8x`, IV rank `0.82`).

- [ ] **Step 1: Write the file (impl + test)**

```rust
use async_trait::async_trait;
use chrono::{TimeZone, Utc};

use crate::domain::entities::market_snapshot::MarketSnapshot;
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::ports::market_data_source::MarketDataSource;

pub struct MockMarketSource;

#[async_trait]
impl MarketDataSource for MockMarketSource {
    fn name(&self) -> &'static str {
        "mock-market"
    }

    async fn snapshot(&self, ticker: &Ticker) -> Result<MarketSnapshot, DomainError> {
        Ok(MarketSnapshot {
            ticker: ticker.clone(),
            as_of: Utc.with_ymd_and_hms(2026, 6, 24, 20, 0, 0).unwrap(),
            last_price: 192.50,
            previous_close: 185.00,
            volume: 95_000_000,
            avg_volume: 52_000_000,
            realized_vol: Some(0.38),
            put_call_ratio: Some(0.7),
            iv_rank: Some(0.82),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn returns_fixture_snapshot() {
        let snap = MockMarketSource.snapshot(&Ticker::parse("AAPL").unwrap()).await.unwrap();
        assert_eq!(snap.last_price, 192.50);
        assert_eq!(snap.iv_rank, Some(0.82));
        assert_eq!(MockMarketSource.name(), "mock-market");
    }
}
```

- [ ] **Step 2: Run test & lint**

Run: `cargo test adapters::market && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: PASS, clean.

- [ ] **Step 3: Commit**

```bash
git add src/adapters/market/mock_market.rs
git commit -m "feat(adapters): add mock market source

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 13: Config — Credentials (secrets) + AppConfig (settings)

**Files:**
- Modify: `src/config/secrets.rs`, `src/config/settings.rs`

**Interfaces:**
- Produces:
  - `Credentials` — `from_env() -> Self` with `Option<SecretString>` fields `reddit_token`, `x_bearer`, `bluesky_app_password`, `market_api_key`. Derives `Debug` (secrecy redacts).
  - `OutputFormat` — enum `{ Table, Json }` (`Copy`).
  - `AppConfig` — `new(ticker: String, reddit: bool, x: bool, bluesky: bool, no_market: bool, limit: usize, format: OutputFormat) -> Self`; public fields `ticker: String`, `enabled_sources: Vec<SourceKind>`, `market_enabled: bool`, `limit: usize`, `format: OutputFormat`, `engine: EngineConfig`. No flags → all three sources.

- [ ] **Step 1: Write `src/config/secrets.rs` (impl + test)**

```rust
use secrecy::SecretString;

#[derive(Debug)]
pub struct Credentials {
    pub reddit_token: Option<SecretString>,
    pub x_bearer: Option<SecretString>,
    pub bluesky_app_password: Option<SecretString>,
    pub market_api_key: Option<SecretString>,
}

impl Credentials {
    pub fn from_env() -> Self {
        Credentials {
            reddit_token: secret_from(std::env::var("OPENINTEL_REDDIT_TOKEN").ok()),
            x_bearer: secret_from(std::env::var("OPENINTEL_X_BEARER").ok()),
            bluesky_app_password: secret_from(std::env::var("OPENINTEL_BLUESKY_APP_PASSWORD").ok()),
            market_api_key: secret_from(std::env::var("OPENINTEL_MARKET_API_KEY").ok()),
        }
    }
}

fn secret_from(value: Option<String>) -> Option<SecretString> {
    value.map(|v| SecretString::new(v.into_boxed_str()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    #[test]
    fn wraps_present_value_and_skips_absent() {
        let some = secret_from(Some("super-token".to_string())).unwrap();
        assert_eq!(some.expose_secret(), "super-token");
        assert!(secret_from(None).is_none());
    }

    #[test]
    fn debug_does_not_leak_secret() {
        let creds = Credentials {
            reddit_token: secret_from(Some("leak-me".to_string())),
            x_bearer: None,
            bluesky_app_password: None,
            market_api_key: None,
        };
        assert!(!format!("{creds:?}").contains("leak-me"));
    }
}
```

- [ ] **Step 2: Write `src/config/settings.rs` (impl + test)**

```rust
use crate::domain::engine::config::EngineConfig;
use crate::domain::values::source_kind::SourceKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Table,
    Json,
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub ticker: String,
    pub enabled_sources: Vec<SourceKind>,
    pub market_enabled: bool,
    pub limit: usize,
    pub format: OutputFormat,
    pub engine: EngineConfig,
}

impl AppConfig {
    pub fn new(
        ticker: String,
        reddit: bool,
        x: bool,
        bluesky: bool,
        no_market: bool,
        limit: usize,
        format: OutputFormat,
    ) -> Self {
        let mut enabled_sources = Vec::new();
        if reddit {
            enabled_sources.push(SourceKind::Reddit);
        }
        if x {
            enabled_sources.push(SourceKind::X);
        }
        if bluesky {
            enabled_sources.push(SourceKind::Bluesky);
        }
        if enabled_sources.is_empty() {
            enabled_sources = vec![SourceKind::Reddit, SourceKind::X, SourceKind::Bluesky];
        }

        AppConfig {
            ticker,
            enabled_sources,
            market_enabled: !no_market,
            limit,
            format,
            engine: EngineConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_flags_enables_all_sources_and_market() {
        let c = AppConfig::new("AAPL".into(), false, false, false, false, 50, OutputFormat::Table);
        assert_eq!(
            c.enabled_sources,
            vec![SourceKind::Reddit, SourceKind::X, SourceKind::Bluesky]
        );
        assert!(c.market_enabled);
    }

    #[test]
    fn single_flag_narrows_sources() {
        let c = AppConfig::new("AAPL".into(), true, false, false, true, 50, OutputFormat::Json);
        assert_eq!(c.enabled_sources, vec![SourceKind::Reddit]);
        assert!(!c.market_enabled);
    }
}
```

- [ ] **Step 3: Run tests & lint**

Run: `cargo test config:: && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: PASS, clean.

- [ ] **Step 4: Commit**

```bash
git add src/config/secrets.rs src/config/settings.rs
git commit -m "feat(config): add env-only Credentials and AppConfig

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 14: CLI args (clap) + mapping to AppConfig

**Files:**
- Modify: `src/cli/args.rs`

**Interfaces:**
- Consumes: `AppConfig`, `OutputFormat`.
- Produces:
  - `Cli` (`Parser`) with `command: Command`.
  - `Command` (`Subcommand`) with `Analyze(AnalyzeArgs)`.
  - `AnalyzeArgs` fields: `ticker: String`, `enable_reddit/enable_x/enable_bluesky: bool`, `no_market: bool`, `limit: usize` (default 50), `format: FormatArg` (default Table).
  - `FormatArg` (`ValueEnum`) `{ Table, Json }`.
  - `to_app_config(&AnalyzeArgs) -> AppConfig`.

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parses_analyze_with_json_format() {
        let cli = Cli::try_parse_from(["openintel", "analyze", "AAPL", "--format", "json"]).unwrap();
        let Command::Analyze(args) = cli.command else { unreachable!() };
        assert_eq!(args.ticker, "AAPL");
        assert_eq!(args.format, FormatArg::Json);
        assert_eq!(args.limit, 50);
    }

    #[test]
    fn maps_no_flags_to_all_sources() {
        let cli = Cli::try_parse_from(["openintel", "analyze", "MSFT"]).unwrap();
        let Command::Analyze(args) = cli.command else { unreachable!() };
        let cfg = to_app_config(&args);
        assert_eq!(cfg.enabled_sources.len(), 3);
        assert!(cfg.market_enabled);
        assert_eq!(cfg.format, crate::config::settings::OutputFormat::Table);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test cli::args`
Expected: FAIL — `Cli` not found.

- [ ] **Step 3: Write the implementation** — prepend above the tests

```rust
use clap::{Parser, Subcommand, ValueEnum};

use crate::config::settings::{AppConfig, OutputFormat};

#[derive(Parser, Debug)]
#[command(
    name = "openintel",
    version,
    about = "Fuse social sentiment with market action into a speculation report"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Analyze a ticker across social + market sources
    Analyze(AnalyzeArgs),
}

#[derive(clap::Args, Debug)]
pub struct AnalyzeArgs {
    /// Ticker symbol, e.g. AAPL
    pub ticker: String,

    #[arg(long)]
    pub enable_reddit: bool,
    #[arg(long)]
    pub enable_x: bool,
    #[arg(long)]
    pub enable_bluesky: bool,

    /// Skip the market snapshot (social-only report)
    #[arg(long)]
    pub no_market: bool,

    /// Posts to fetch per source
    #[arg(long, default_value_t = 50)]
    pub limit: usize,

    #[arg(long, value_enum, default_value_t = FormatArg::Table)]
    pub format: FormatArg,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormatArg {
    Table,
    Json,
}

pub fn to_app_config(args: &AnalyzeArgs) -> AppConfig {
    let format = match args.format {
        FormatArg::Table => OutputFormat::Table,
        FormatArg::Json => OutputFormat::Json,
    };
    AppConfig::new(
        args.ticker.clone(),
        args.enable_reddit,
        args.enable_x,
        args.enable_bluesky,
        args.no_market,
        args.limit,
        format,
    )
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test cli::args && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: PASS, clean.

- [ ] **Step 5: Commit**

```bash
git add src/cli/args.rs
git commit -m "feat(cli): add clap args and AppConfig mapping

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 15: cli::run — orchestration + rendering

**Files:**
- Modify: `src/cli/run.rs`

**Interfaces:**
- Consumes: `AppConfig`, `OutputFormat`, mocks, `LexiconAnalyzer`, `SpeculationEngine`, ports, `Ticker`, `SpeculationReport`, `DomainError`, `SourceKind`.
- Produces:
  - `pub const DISCLAIMER: &str`.
  - `pub async fn analyze(config: &AppConfig) -> Result<(SpeculationReport, String), DomainError>` — builds enabled sources, concurrent fetch (per-source failure non-fatal → note), optional market, errors `NoData` when no posts and no market, runs analyzer + engine, returns report + rendered string.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::settings::{AppConfig, OutputFormat};
    use crate::domain::values::speculation::Alignment;

    fn config(no_market: bool, format: OutputFormat) -> AppConfig {
        AppConfig::new("AAPL".into(), false, false, false, no_market, 50, format)
    }

    #[tokio::test]
    async fn full_run_confirms_bullish_with_market() {
        let (report, rendered) = analyze(&config(false, OutputFormat::Json)).await.unwrap();
        assert!(report.market.is_some());
        assert_eq!(report.fusion.alignment, Alignment::ConfirmingBullish);
        assert!(rendered.contains("Not financial advice"));
        assert!(rendered.contains("speculation_index"));
    }

    #[tokio::test]
    async fn no_market_run_is_quiet() {
        let (report, _) = analyze(&config(true, OutputFormat::Table)).await.unwrap();
        assert!(report.market.is_none());
        assert_eq!(report.fusion.alignment, Alignment::Quiet);
    }

    #[tokio::test]
    async fn table_output_has_sections_and_disclaimer() {
        let (_, rendered) = analyze(&config(false, OutputFormat::Table)).await.unwrap();
        assert!(rendered.contains("SOCIAL"));
        assert!(rendered.contains("FUSION"));
        assert!(rendered.contains("Not financial advice"));
    }

    #[tokio::test]
    async fn invalid_ticker_errors() {
        let cfg = AppConfig::new("$$$".into(), false, false, false, false, 50, OutputFormat::Table);
        assert!(analyze(&cfg).await.is_err());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test cli::run`
Expected: FAIL — `analyze` not found.

- [ ] **Step 3: Write the implementation** — prepend above the tests

```rust
use chrono::Utc;
use futures::future::join_all;

use crate::adapters::analyzer::lexicon::LexiconAnalyzer;
use crate::adapters::market::mock_market::MockMarketSource;
use crate::adapters::sources::mock_bluesky::MockBlueskySource;
use crate::adapters::sources::mock_reddit::MockRedditSource;
use crate::adapters::sources::mock_x::MockXSource;
use crate::config::settings::{AppConfig, OutputFormat};
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

pub const DISCLAIMER: &str = "Not financial advice. OpenIntel is a research/screening tool; \
markets are risky and social data is easily manipulated. Do your own diligence.";

fn build_sources(config: &AppConfig) -> Vec<Box<dyn SocialDataSource>> {
    config
        .enabled_sources
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

pub async fn analyze(config: &AppConfig) -> Result<(SpeculationReport, String), DomainError> {
    let ticker = Ticker::parse(&config.ticker)?;
    let sources = build_sources(config);

    // Concurrent social fetch; a single source failing is non-fatal.
    let fetches = sources
        .iter()
        .map(|source| async move { (source.kind(), source.fetch(&ticker, config.limit).await) });
    let results = join_all(fetches).await;

    let mut posts: Vec<SocialPost> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    for (kind, result) in results {
        match result {
            Ok(mut fetched) => posts.append(&mut fetched),
            Err(e) => notes.push(format!("source {} failed: {e}", kind.as_str())),
        }
    }

    let market: Option<MarketSnapshot> = if config.market_enabled {
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
        SpeculationEngine::aggregate(&ticker, &posts, &signals, market.as_ref(), now, &config.engine)?;

    // Prepend orchestration notes (source/market failures) ahead of engine notes.
    let engine_notes = std::mem::take(&mut report.fusion.notes);
    report.fusion.notes = notes.into_iter().chain(engine_notes).collect();

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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test cli::run && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: PASS (4 tests), clean.

- [ ] **Step 5: Commit**

```bash
git add src/cli/run.rs
git commit -m "feat(cli): add analyze orchestration and table/json rendering

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 16: main.rs composition root

**Files:**
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: `Cli`, `Command`, `to_app_config`, `Credentials`, `analyze`.
- Produces: a `#[tokio::main]` binary that parses args, loads credentials (unused by mocks), runs `analyze`, prints the rendered report, and maps errors to a non-zero exit code.

- [ ] **Step 1: Replace `src/main.rs`**

```rust
use std::process::ExitCode;

use clap::Parser;

use openintel::cli::args::{to_app_config, Cli, Command};
use openintel::cli::run::analyze;
use openintel::config::secrets::Credentials;

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    // Loaded for future real adapters; mock adapters ignore credentials.
    let _credentials = Credentials::from_env();

    match cli.command {
        Command::Analyze(args) => {
            let config = to_app_config(&args);
            match analyze(&config).await {
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
    }
}
```

- [ ] **Step 2: Build and smoke-test the binary**

Run: `cargo build && cargo run --quiet -- analyze AAPL`
Expected: prints a table report containing `SOCIAL`, `FUSION`, `alignment: ConfirmingBullish`, and the disclaimer; exit code 0.

- [ ] **Step 3: Smoke-test JSON + error paths**

Run: `cargo run --quiet -- analyze AAPL --format json | head -1`
Expected: `{` (pretty JSON start).

Run: `cargo run --quiet -- analyze '$$$'; echo "exit=$?"`
Expected: `error: invalid ticker: $$$` on stderr, `exit=1`.

- [ ] **Step 4: Lint & format**

Run: `cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add src/main.rs
git commit -m "feat(cli): wire composition root in main

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 17: Integration tests

**Files:**
- Create: `tests/analyze_flow.rs`

**Interfaces:**
- Consumes: public lib API `openintel::cli::run::analyze`, `openintel::config::settings::{AppConfig, OutputFormat}`, `openintel::domain::values::speculation::Alignment`.

- [ ] **Step 1: Write `tests/analyze_flow.rs`**

```rust
use openintel::cli::run::analyze;
use openintel::config::settings::{AppConfig, OutputFormat};
use openintel::domain::values::speculation::Alignment;

fn cfg(reddit: bool, x: bool, bluesky: bool, no_market: bool) -> AppConfig {
    AppConfig::new("AAPL".into(), reddit, x, bluesky, no_market, 50, OutputFormat::Json)
}

#[tokio::test]
async fn end_to_end_all_sources_with_market() {
    let (report, json) = analyze(&cfg(false, false, false, false)).await.unwrap();
    // 4 + 3 + 3 mock posts across reddit/x/bluesky (>= min_sample of 10)
    assert_eq!(report.social.total_mentions, 10);
    assert_eq!(report.fusion.alignment, Alignment::ConfirmingBullish);
    assert!(report.market.is_some());
    assert!(json.contains("\"alignment\": \"confirming_bullish\""));
    assert!(json.contains("Not financial advice"));
}

#[tokio::test]
async fn single_source_only() {
    let (report, _) = analyze(&cfg(true, false, false, false)).await.unwrap();
    assert_eq!(report.social.total_mentions, 4); // reddit fixtures only
}

#[tokio::test]
async fn social_only_when_market_disabled() {
    let (report, _) = analyze(&cfg(false, false, false, true)).await.unwrap();
    assert!(report.market.is_none());
    assert_eq!(report.fusion.alignment, Alignment::Quiet);
}
```

- [ ] **Step 2: Run the integration tests**

Run: `cargo test --test analyze_flow`
Expected: PASS (3 tests).

> Note on the JSON assertion: `serde_json::to_string_pretty` renders object entries as `"key": value` (space after colon). The `"\"alignment\": \"confirming_bullish\""` substring matches that pretty format. If the executor switched to compact JSON, drop the space.

- [ ] **Step 3: Full suite + lint**

Run: `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: PASS (all unit + integration tests), clean.

- [ ] **Step 4: Commit**

```bash
git add tests/analyze_flow.rs
git commit -m "test: end-to-end analyze flow over mock adapters

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 18: README (usage + extensibility playbook)

**Files:**
- Create/replace: `README.md`

**Interfaces:**
- Consumes: nothing (documentation).

- [ ] **Step 1: Write `README.md`**

````markdown
# OpenIntel

Security-first CLI that fuses social-media chatter with market action into a **speculation report** — a crowding & divergence detector for a ticker.

> **Not financial advice.** OpenIntel is a research/screening tool. Social data is noisy and easily manipulated. Do your own diligence.

## Usage

```bash
# All social sources + market snapshot (default)
openintel analyze AAPL

# Narrow to specific sources
openintel analyze AAPL --enable-reddit --enable-x

# Social only, JSON output
openintel analyze AAPL --no-market --format json
```

| Flag | Meaning |
|---|---|
| `--enable-reddit/-x/-bluesky` | Restrict to these sources (none given → all enabled) |
| `--no-market` | Skip the market snapshot (social-only report) |
| `--limit <N>` | Posts per source (default 50) |
| `--format table\|json` | Output format (default table) |

## What it computes

- **net sentiment** — mean per-post polarity `[-1, 1]`
- **speculation index** — share of posts using options/leverage jargon
- **rvol / pct change** — volume vs average, day move
- **crowding** — blended speculation + RVOL + IV rank `[0, 1]`
- **alignment** — `ConfirmingBullish/Bearish`, `Diverging`, or `Quiet`

## Architecture

Hexagonal (ports & adapters). The domain is pure and synchronous; IO and the clock live at the edge.

- `domain/` — entities, value objects, the pure `SpeculationEngine`, and port traits.
- `adapters/` — `LexiconAnalyzer` + mock data sources.
- `config/` — env-only secrets (`secrecy`) and runtime settings.
- `cli/` — clap args, orchestration, rendering.

Secrets come only from environment variables (`OPENINTEL_REDDIT_TOKEN`, `OPENINTEL_X_BEARER`, `OPENINTEL_BLUESKY_APP_PASSWORD`, `OPENINTEL_MARKET_API_KEY`), wrapped in `SecretString` — never logged or written to disk.

## Extending

**Add a social source** (e.g. real Reddit):
1. New struct in `src/adapters/sources/`, `impl SocialDataSource`.
2. Add a `SourceKind` variant in `src/domain/values/source_kind.rs`.
3. Add one arm to `build_sources` in `src/cli/run.rs`.

**Add a market source** (e.g. Yahoo Finance):
1. New struct in `src/adapters/market/`, `impl MarketDataSource`.
2. Select it in `cli::run`.

**Swap the analyzer** (lexicon → LLM/ML):
1. New struct in `src/adapters/analyzer/`, `impl PostAnalyzer`. No engine change.

## Development

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```
````

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "docs: README with usage and extensibility playbook

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage:** §3 layout → Tasks 1–18. §4 entities/values/ports → Tasks 2–7. §4 engine signature (injected `now`, `Result`, `signals`) → Task 9. §5 metrics/fusion/renormalized crowding/`σ` vs `τ`/edges → Task 9 (tests cover empty, no-market, mismatch, rvol guard, renormalization, confirming, diverging). §6 env-only secrets + redaction → Task 13. §6 disclaimer in renderer → Task 15. §7 CLI surface → Task 14. §8 execution flow + non-fatal source failure + `NoData` → Task 15. §9 deps → Task 1. §10 testing (unit + integration + fixed timestamps) → Tasks 2–17. §11 extensibility playbook → Task 18 README. §12 responsible-use framing → Task 18 + DISCLAIMER. Deferred items (real adapters, velocity/history, options depth, bot filtering) intentionally out of scope.

**Placeholder scan:** No `TODO`/`TBD`/"handle errors appropriately". Every code step shows complete code; every run step shows the exact command + expected output.

**Type consistency:** `aggregate(ticker, posts, signals, market, now, cfg) -> Result<SpeculationReport, DomainError>` identical in Task 9 interface, impl, and Task 15 call site. `PostSignal { polarity, speculative }` consistent across Tasks 3, 7, 9, 10. `AppConfig::new(ticker, reddit, x, bluesky, no_market, limit, format)` consistent across Tasks 13, 14. `SourceKind::as_str` used in Tasks 3, 15. `DomainError::{AnalyzerMismatch, NoData, InvalidTicker}` consistent across Tasks 2, 9, 15. `OutputFormat`/`FormatArg` mapping consistent in Task 14. `LexiconAnalyzer::new` consistent in Tasks 10, 15. `MockRedditSource`/`MockXSource`/`MockBlueskySource`/`MockMarketSource` consistent in Tasks 11, 12, 15.
