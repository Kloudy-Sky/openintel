# risk_frame Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `openintel risk <TICKER> --budget N` + MCP `risk_frame`: deterministic ATR(14) stop, whole-share size capping a stop-out at the budget, 1R/2R/3R references. A calculator — never advises.

**Architecture:** Pure domain math (`domain/risk.rs` over a new `Bar` value) → new small `BarSource` port (Yahoo parser starts keeping high/low from the SAME request; `MarketDataSource`/mocks untouched) → `application/risk.rs` (validation, clamps, clock at edge) → CLI leaf + MCP tool.

**Tech Stack:** Rust; existing Yahoo v8 chart adapter; clap; rmcp.

**Spec:** `docs/superpowers/specs/2026-07-16-risk-frame-design.md` — math and copy verbatim from it.

## Global Constraints

- **Math pinned:** `TR = max(high−low, |high−prev_close|, |low−prev_close|)`; ATR(14) = simple mean of last 14 TRs, requires ≥ 15 bars; stop = `entry − k·ATR` (long) / `entry + k·ATR` (short); `k = stop_multiple.clamp(0.5, 5.0)`, default 2.0; `shares = floor(budget / (k·ATR))` (whole shares); `max_loss_usd = shares × k·ATR` (the ACTUAL capped number); targets `entry ± n·k·ATR` for n = 1,2,3 (direction-signed); `notional = shares × entry`.
- **`shares == 0` is a valid result** (note text: `budget too small for one share at this stop distance`), not an error.
- **Errors are clean messages, never NaN/panic:** < 15 bars → `not enough history for ATR(14)`; ATR ≤ 0 → `flat price history — ATR is zero`; long stop ≤ 0 → `stop below zero — use a smaller multiple`; budget/entry must be finite and > 0.
- **Pure domain:** `domain/risk.rs` is sync, no clock, no IO; `generated_at` stamped by the application layer.
- **Additive only:** `MarketDataSource`, `MarketSnapshot`, `mock_market`, and every existing test untouched; the Yahoo parser gains `high`/`low` arrays and a `parse_bars`; `BarSource` is a separate port implemented by `YahooMarketSource` (the MCP server reuses its existing `market` field — no new server field).
- **Calculator framing:** CLI table ends with `risk_frame is a calculator, not advice — it never recommends taking a trade.` above the standard DISCLAIMER; the MCP description is the spec's contract verbatim.
- **stdout discipline:** `cli/risk.rs` returns Strings; only `main.rs` prints.
- **Hermetic tests** + one `#[ignore]`d live Yahoo bars test (keyless, free).
- **Every commit green:** `cargo build --all-targets && cargo test && cargo clippy -- -D warnings && cargo fmt -- --check`.

---

### Task 1: Pure domain — `Bar` + `domain/risk.rs`

**Files:**
- Create: `src/domain/values/bar.rs`, `src/domain/risk.rs`
- Modify: `src/domain/values/mod.rs`, `src/domain/mod.rs` (registrations, keep alphabetical)

**Interfaces:**
- Produces (later tasks rely on exactly these):
  - `Bar { pub high: f64, pub low: f64, pub close: f64 }` (`Debug, Clone, Copy, PartialEq`)
  - `Direction { Long, Short }` (`Debug, Clone, Copy, PartialEq, Eq, Serialize`, `#[serde(rename_all = "lowercase")]`)
  - `RiskFrame { ticker: String, direction: Direction, entry: f64, atr: f64, stop_multiple: f64, stop: f64, risk_per_share: f64, shares: u64, max_loss_usd: f64, budget_usd: f64, targets: [f64; 3], notional_usd: f64, bars_used: usize, note: Option<String>, generated_at: DateTime<Utc> }` (`Debug, Clone, Serialize`)
  - `pub fn true_ranges(bars: &[Bar]) -> Vec<f64>`
  - `pub fn atr(bars: &[Bar], period: usize) -> Option<f64>`
  - `pub fn frame(ticker: &str, bars: &[Bar], direction: Direction, entry: f64, budget_usd: f64, stop_multiple: f64, generated_at: DateTime<Utc>) -> Result<RiskFrame, DomainError>`
  - `pub const ATR_PERIOD: usize = 14;`

- [ ] **Step 1: `src/domain/values/bar.rs`**

```rust
/// One daily OHLC bar (open omitted — nothing here needs it).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bar {
    pub high: f64,
    pub low: f64,
    pub close: f64,
}
```

Register `pub mod bar;` in `src/domain/values/mod.rs`.

- [ ] **Step 2: `src/domain/risk.rs`**

```rust
//! Deterministic per-trade risk math: ATR(14) stop, budget-capped whole-share
//! size, R-multiple reference levels. Pure and synchronous — a calculator,
//! never an advisor. The clock is stamped by the application layer.

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::domain::error::DomainError;
use crate::domain::values::bar::Bar;

pub const ATR_PERIOD: usize = 14;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Long,
    Short,
}

#[derive(Debug, Clone, Serialize)]
pub struct RiskFrame {
    pub ticker: String,
    pub direction: Direction,
    pub entry: f64,
    pub atr: f64,
    pub stop_multiple: f64,
    pub stop: f64,
    pub risk_per_share: f64,
    pub shares: u64,
    /// shares × risk_per_share — the ACTUAL capped loss (≤ budget_usd).
    pub max_loss_usd: f64,
    pub budget_usd: f64,
    /// 1R / 2R / 3R price levels (direction-signed reference exits).
    pub targets: [f64; 3],
    pub notional_usd: f64,
    pub bars_used: usize,
    pub note: Option<String>,
    pub generated_at: DateTime<Utc>,
}

fn fail(message: impl Into<String>) -> DomainError {
    DomainError::SourceFailure {
        name: "risk".into(),
        message: message.into(),
    }
}

/// True ranges for bars[1..] (each needs the previous close).
pub fn true_ranges(bars: &[Bar]) -> Vec<f64> {
    bars.windows(2)
        .map(|w| {
            let prev_close = w[0].close;
            let b = w[1];
            (b.high - b.low)
                .max((b.high - prev_close).abs())
                .max((b.low - prev_close).abs())
        })
        .collect()
}

/// Simple mean of the last `period` true ranges. None if history is too thin.
pub fn atr(bars: &[Bar], period: usize) -> Option<f64> {
    let trs = true_ranges(bars);
    if trs.len() < period || period == 0 {
        return None;
    }
    let tail = &trs[trs.len() - period..];
    Some(tail.iter().sum::<f64>() / period as f64)
}

pub fn frame(
    ticker: &str,
    bars: &[Bar],
    direction: Direction,
    entry: f64,
    budget_usd: f64,
    stop_multiple: f64,
    generated_at: DateTime<Utc>,
) -> Result<RiskFrame, DomainError> {
    if !(budget_usd.is_finite() && budget_usd > 0.0) {
        return Err(fail("budget must be a positive number"));
    }
    if !(entry.is_finite() && entry > 0.0) {
        return Err(fail("entry must be a positive price"));
    }
    let stop_multiple = stop_multiple.clamp(0.5, 5.0);
    let atr = atr(bars, ATR_PERIOD)
        .ok_or_else(|| fail(format!("not enough history for ATR({ATR_PERIOD})")))?;
    if atr <= 0.0 {
        return Err(fail("flat price history — ATR is zero"));
    }

    let risk_per_share = stop_multiple * atr;
    let stop = match direction {
        Direction::Long => entry - risk_per_share,
        Direction::Short => entry + risk_per_share,
    };
    if stop <= 0.0 {
        return Err(fail("stop below zero — use a smaller multiple"));
    }

    let shares = (budget_usd / risk_per_share).floor() as u64;
    let note = (shares == 0)
        .then(|| "budget too small for one share at this stop distance".to_string());
    let signed = |n: f64| match direction {
        Direction::Long => entry + n * risk_per_share,
        Direction::Short => entry - n * risk_per_share,
    };

    Ok(RiskFrame {
        ticker: ticker.to_string(),
        direction,
        entry,
        atr,
        stop_multiple,
        stop,
        risk_per_share,
        shares,
        max_loss_usd: shares as f64 * risk_per_share,
        budget_usd,
        targets: [signed(1.0), signed(2.0), signed(3.0)],
        notional_usd: shares as f64 * entry,
        bars_used: bars.len(),
        note,
        generated_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 16, 0, 0, 0).unwrap()
    }

    fn bar(high: f64, low: f64, close: f64) -> Bar {
        Bar { high, low, close }
    }

    /// 16 bars: prev_close 100, then 15 identical bars with TR dominated by
    /// a gap on bar 2 (|high − prev_close| = 8 > high − low = 4).
    fn bars() -> Vec<Bar> {
        let mut v = vec![bar(101.0, 99.0, 100.0)];
        v.push(bar(108.0, 104.0, 106.0)); // gap day: TR = 108-100 = 8
        for _ in 0..14 {
            v.push(bar(108.0, 104.0, 106.0)); // TR = high-low = 4
        }
        v
    }

    #[test]
    fn true_range_counts_gaps() {
        let trs = true_ranges(&bars());
        assert_eq!(trs.len(), 15);
        assert!((trs[0] - 8.0).abs() < 1e-12); // gap day
        assert!((trs[1] - 4.0).abs() < 1e-12);
    }

    #[test]
    fn atr_is_mean_of_last_period() {
        // last 14 TRs are all 4.0 (the gap day falls outside the window)
        assert!((atr(&bars(), 14).unwrap() - 4.0).abs() < 1e-12);
        assert!(atr(&bars()[..14], 14).is_none()); // 13 TRs < 14
    }

    #[test]
    fn long_frame_math() {
        let f = frame("NVDA", &bars(), Direction::Long, 106.0, 200.0, 2.0, at()).unwrap();
        assert!((f.atr - 4.0).abs() < 1e-12);
        assert!((f.risk_per_share - 8.0).abs() < 1e-12);
        assert!((f.stop - 98.0).abs() < 1e-12);
        assert_eq!(f.shares, 25); // floor(200 / 8)
        assert!((f.max_loss_usd - 200.0).abs() < 1e-12);
        assert!(f.max_loss_usd <= f.budget_usd);
        assert!((f.targets[0] - 114.0).abs() < 1e-12);
        assert!((f.targets[2] - 130.0).abs() < 1e-12);
        assert!((f.notional_usd - 2650.0).abs() < 1e-12);
        assert!(f.note.is_none());
    }

    #[test]
    fn short_frame_flips_signs() {
        let f = frame("NVDA", &bars(), Direction::Short, 106.0, 100.0, 1.0, at()).unwrap();
        assert!((f.stop - 110.0).abs() < 1e-12);
        assert!((f.targets[0] - 102.0).abs() < 1e-12);
        assert_eq!(f.shares, 25); // floor(100 / 4)
    }

    #[test]
    fn zero_shares_is_valid_with_note_and_max_loss_zero() {
        let f = frame("NVDA", &bars(), Direction::Long, 106.0, 5.0, 2.0, at()).unwrap();
        assert_eq!(f.shares, 0);
        assert_eq!(f.max_loss_usd, 0.0);
        assert!(f.note.as_deref().unwrap().contains("too small"));
    }

    #[test]
    fn clamps_and_errors() {
        // multiple clamped up from 0.1 to 0.5
        let f = frame("N", &bars(), Direction::Long, 106.0, 100.0, 0.1, at()).unwrap();
        assert!((f.stop_multiple - 0.5).abs() < 1e-12);
        // multiple clamped down from 9 to 5
        let f = frame("N", &bars(), Direction::Long, 106.0, 100.0, 9.0, at()).unwrap();
        assert!((f.stop_multiple - 5.0).abs() < 1e-12);
        assert!(frame("N", &bars(), Direction::Long, 106.0, 0.0, 2.0, at()).is_err());
        assert!(frame("N", &bars(), Direction::Long, -1.0, 100.0, 2.0, at()).is_err());
        assert!(frame("N", &bars()[..10], Direction::Long, 106.0, 100.0, 2.0, at()).is_err());
        // long stop below zero: entry 3, k=5, atr 4 -> stop = -17
        assert!(frame("N", &bars(), Direction::Long, 3.0, 100.0, 5.0, at()).is_err());
        // flat history -> ATR 0 -> error
        let flat = vec![bar(100.0, 100.0, 100.0); 16];
        assert!(frame("N", &flat, Direction::Long, 100.0, 100.0, 2.0, at()).is_err());
    }
}
```

Register `pub mod risk;` in `src/domain/mod.rs`.

- [ ] **Step 3: Verify + commit**

Run: `cargo build --all-targets && cargo test && cargo clippy -- -D warnings && cargo fmt`
Expected: green (7 new tests).

```bash
git add src/domain/values/bar.rs src/domain/risk.rs src/domain/values/mod.rs src/domain/mod.rs
git commit -m "feat(risk): pure ATR/true-range/frame math + Bar value

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 2: `BarSource` port + Yahoo bars

**Files:**
- Create: `src/domain/ports/bar_source.rs`
- Modify: `src/domain/ports/mod.rs`, `src/adapters/market/yahoo/response.rs`, `src/adapters/market/yahoo/mod.rs`

**Interfaces:**
- Consumes: Task 1's `Bar`.
- Produces: `#[async_trait] pub trait BarSource: Send + Sync { async fn bars(&self, ticker: &Ticker) -> Result<Vec<Bar>, DomainError>; }`; `impl BarSource for YahooMarketSource`; `pub(crate) fn parse_bars(body: &str) -> Result<Vec<Bar>, DomainError>` in yahoo::response.

- [ ] **Step 1: `src/domain/ports/bar_source.rs`**

```rust
use async_trait::async_trait;

use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::values::bar::Bar;

/// Daily OHLC bars for risk math (ATR). Kept separate from
/// `MarketDataSource` so snapshot consumers and mocks are untouched.
#[async_trait]
pub trait BarSource: Send + Sync {
    async fn bars(&self, ticker: &Ticker) -> Result<Vec<Bar>, DomainError>;
}
```

Register `pub mod bar_source;` in `src/domain/ports/mod.rs`.

- [ ] **Step 2: Yahoo parser keeps high/low**

In `src/adapters/market/yahoo/response.rs`, extend `Quote`:

```rust
struct Quote {
    #[serde(default)]
    close: Vec<Option<f64>>,
    #[serde(default)]
    volume: Vec<Option<u64>>,
    #[serde(default)]
    high: Vec<Option<f64>>,
    #[serde(default)]
    low: Vec<Option<f64>>,
}
```

(Match the file's existing field attributes/ordering; `close`/`volume` handling is untouched.) Add, alongside `parse_snapshot`:

```rust
/// OHLC bars from the same chart response `parse_snapshot` reads. Rows with
/// any missing leg (Yahoo emits nulls for halts/partial days) are skipped.
pub(crate) fn parse_bars(body: &str) -> Result<Vec<crate::domain::values::bar::Bar>, DomainError> {
    let chart: ChartResponse = serde_json::from_str(body)
        .map_err(|e| yahoo_fail(format!("malformed response: {e}")))?;
    let result = first_result(chart)?; // reuse the file's existing extraction path
    let quote = extract_quote(result)?; // ditto
    let bars = quote
        .high
        .iter()
        .zip(quote.low.iter())
        .zip(quote.close.iter())
        .filter_map(|((h, l), c)| {
            Some(crate::domain::values::bar::Bar {
                high: (*h)?,
                low: (*l)?,
                close: (*c)?,
            })
        })
        .collect();
    Ok(bars)
}
```

**Adaptation note:** `first_result`/`extract_quote`/`yahoo_fail` name whatever helpers the file actually uses to get from `ChartResponse` to `Quote` in `parse_snapshot` — reuse those exact paths (extract a shared helper if the logic is inline) rather than duplicating the extraction. Behavior as specified; report the shape you chose.

Tests (same file's test module, reusing its fixture style):

```rust
    #[test]
    fn parse_bars_zips_and_skips_null_legs() {
        let body = r#"{"chart":{"result":[{"meta":{},"indicators":{"quote":[{
            "close":[100.0,106.0,null,107.0],
            "volume":[1,1,1,1],
            "high":[101.0,108.0,109.0,null],
            "low":[99.0,104.0,105.0,106.0]
        }]}}],"error":null}}"#;
        let bars = parse_bars(body).unwrap();
        assert_eq!(bars.len(), 2); // rows 2 (null close) and 3 (null high) skipped
        assert_eq!(bars[0].high, 101.0);
        assert_eq!(bars[1].close, 106.0);
    }

    #[test]
    fn parse_bars_malformed_and_empty() {
        assert!(parse_bars("nope").is_err());
    }
```

(Adjust the fixture JSON shape to match the file's existing fixtures — the meta/indicators nesting must mirror what `parse_snapshot`'s tests use.)

- [ ] **Step 3: `impl BarSource for YahooMarketSource`** in `src/adapters/market/yahoo/mod.rs`

Reuse the existing chart-request path (same URL builder / HTTP handling `snapshot` uses — extract a `fetch_chart_body(&self, ticker) -> Result<String, DomainError>` helper if the request is currently inline, and have BOTH `snapshot` and `bars` call it):

```rust
#[async_trait]
impl crate::domain::ports::bar_source::BarSource for YahooMarketSource {
    async fn bars(
        &self,
        ticker: &Ticker,
    ) -> Result<Vec<crate::domain::values::bar::Bar>, DomainError> {
        let body = self.fetch_chart_body(ticker).await?;
        response::parse_bars(&body)
    }
}
```

Add one `#[ignore]`d live test:

```rust
    #[tokio::test]
    #[ignore = "hits live Yahoo (keyless, free); run with --ignored"]
    async fn live_bars_have_sane_ohlc() {
        let src = YahooMarketSource::new().unwrap();
        let bars = crate::domain::ports::bar_source::BarSource::bars(
            &src,
            &Ticker::parse("AAPL").unwrap(),
        )
        .await
        .unwrap();
        assert!(bars.len() >= 15);
        for b in &bars {
            assert!(b.high >= b.low);
        }
    }
```

- [ ] **Step 4: Verify + commit**

Run: `cargo build --all-targets && cargo test && cargo clippy -- -D warnings && cargo fmt`

```bash
git add src/domain/ports/bar_source.rs src/domain/ports/mod.rs src/adapters/market/yahoo/
git commit -m "feat(market): BarSource port — Yahoo parser keeps high/low for ATR

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 3: `application/risk.rs`

**Files:**
- Create: `src/application/risk.rs`
- Modify: `src/application/mod.rs`

**Interfaces:**
- Consumes: Tasks 1–2 (`domain::risk::{frame, Direction, RiskFrame}`, `BarSource`).
- Produces: `pub async fn risk_frame(ticker_raw: &str, direction: Direction, budget_usd: f64, stop_multiple: Option<f64>, entry: Option<f64>, bars: &dyn BarSource, now: DateTime<Utc>) -> Result<RiskFrame, DomainError>`; `pub const DEFAULT_STOP_MULTIPLE: f64 = 2.0;`

- [ ] **Step 1: `src/application/risk.rs`**

```rust
use chrono::{DateTime, Utc};

use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::ports::bar_source::BarSource;
use crate::domain::risk::{frame, Direction, RiskFrame};

pub const DEFAULT_STOP_MULTIPLE: f64 = 2.0;

/// Fetch bars, default the entry to the last close, run the pure frame math.
/// Clock injected at this edge; all validation errors are clean messages.
pub async fn risk_frame(
    ticker_raw: &str,
    direction: Direction,
    budget_usd: f64,
    stop_multiple: Option<f64>,
    entry: Option<f64>,
    bars: &dyn BarSource,
    now: DateTime<Utc>,
) -> Result<RiskFrame, DomainError> {
    let ticker = Ticker::parse(ticker_raw)?;
    let history = bars.bars(&ticker).await?;
    let entry = match entry {
        Some(e) => e,
        None => {
            history
                .last()
                .ok_or_else(|| DomainError::SourceFailure {
                    name: "risk".into(),
                    message: "no price history".into(),
                })?
                .close
        }
    };
    frame(
        ticker.as_str(),
        &history,
        direction,
        entry,
        budget_usd,
        stop_multiple.unwrap_or(DEFAULT_STOP_MULTIPLE),
        now,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::values::bar::Bar;
    use async_trait::async_trait;
    use chrono::TimeZone;

    fn at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 16, 0, 0, 0).unwrap()
    }

    struct FixedBars(Vec<Bar>);

    #[async_trait]
    impl BarSource for FixedBars {
        async fn bars(&self, _t: &Ticker) -> Result<Vec<Bar>, DomainError> {
            Ok(self.0.clone())
        }
    }

    fn history() -> Vec<Bar> {
        let mut v = vec![Bar { high: 101.0, low: 99.0, close: 100.0 }];
        for _ in 0..15 {
            v.push(Bar { high: 108.0, low: 104.0, close: 106.0 });
        }
        v
    }

    #[tokio::test]
    async fn defaults_entry_to_last_close_and_multiple_to_two() {
        let f = risk_frame("nvda", Direction::Long, 200.0, None, None, &FixedBars(history()), at())
            .await
            .unwrap();
        assert_eq!(f.ticker, "NVDA");
        assert!((f.entry - 106.0).abs() < 1e-12);
        assert!((f.stop_multiple - 2.0).abs() < 1e-12);
        assert_eq!(f.generated_at, at());
    }

    #[tokio::test]
    async fn entry_override_and_errors_propagate() {
        let f = risk_frame(
            "NVDA",
            Direction::Short,
            100.0,
            Some(1.0),
            Some(110.0),
            &FixedBars(history()),
            at(),
        )
        .await
        .unwrap();
        assert!((f.entry - 110.0).abs() < 1e-12);
        assert!(
            risk_frame("$$$", Direction::Long, 100.0, None, None, &FixedBars(history()), at())
                .await
                .is_err()
        );
        assert!(
            risk_frame("NVDA", Direction::Long, 100.0, None, None, &FixedBars(vec![]), at())
                .await
                .is_err()
        );
    }
}
```

Register `pub mod risk;` in `src/application/mod.rs`.

- [ ] **Step 2: Verify + commit**

```bash
git add src/application/risk.rs src/application/mod.rs
git commit -m "feat(risk): application orchestration — bars fetch, entry default, clock at edge

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 4: CLI — `openintel risk`

**Files:**
- Create: `src/cli/risk.rs`
- Modify: `src/cli/args.rs`, `src/cli/mod.rs`, `src/main.rs`

**Interfaces:**
- Consumes: Task 3's `application::risk::risk_frame`; `YahooMarketSource` (implements `BarSource`); existing `FormatArg`, `DISCLAIMER`.
- Produces: `Command::Risk(RiskArgs)`; `pub async fn run(args: &RiskArgs) -> Result<String, DomainError>` in `cli::risk`.

- [ ] **Step 1: `src/cli/args.rs`** — after `Pulse`:

```rust
    /// Deterministic risk math for one trade idea: ATR stop, budget-capped size, R targets
    Risk(RiskArgs),
```

and below `PulseArgs`:

```rust
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectionArg {
    Long,
    Short,
}

#[derive(clap::Args, Debug)]
pub struct RiskArgs {
    /// Ticker symbol, e.g. NVDA
    pub ticker: String,

    /// Per-trade risk budget in USD — the most a stop-out may lose
    #[arg(long)]
    pub budget: f64,

    #[arg(long, value_enum, default_value_t = DirectionArg::Long)]
    pub direction: DirectionArg,

    /// Stop distance in ATR multiples (0.5-5)
    #[arg(long = "stop-mult", default_value_t = 2.0)]
    pub stop_mult: f64,

    /// Entry price override (default: last close)
    #[arg(long)]
    pub entry: Option<f64>,

    #[arg(long, value_enum, default_value_t = FormatArg::Table)]
    pub format: FormatArg,
}
```

Tests:

```rust
    #[test]
    fn parses_risk_args() {
        let cli = Cli::try_parse_from([
            "openintel", "risk", "NVDA", "--budget", "200", "--direction", "short",
            "--stop-mult", "1.5",
        ])
        .unwrap();
        let Command::Risk(args) = cli.command else {
            panic!("expected risk command");
        };
        assert_eq!(args.ticker, "NVDA");
        assert_eq!(args.budget, 200.0);
        assert_eq!(args.direction, DirectionArg::Short);
        assert_eq!(args.stop_mult, 1.5);
        assert!(args.entry.is_none());
    }

    #[test]
    fn risk_requires_budget() {
        assert!(Cli::try_parse_from(["openintel", "risk", "NVDA"]).is_err());
    }
```

- [ ] **Step 2: `src/cli/risk.rs`**

```rust
//! CLI leaf for `openintel risk` — returns rendered Strings; main prints.

use chrono::Utc;

use crate::adapters::market::yahoo::YahooMarketSource;
use crate::application::DISCLAIMER;
use crate::cli::args::{DirectionArg, FormatArg, RiskArgs};
use crate::domain::error::DomainError;
use crate::domain::risk::{Direction, RiskFrame};

const CALCULATOR_LINE: &str =
    "risk_frame is a calculator, not advice — it never recommends taking a trade.";

pub async fn run(args: &RiskArgs) -> Result<String, DomainError> {
    let direction = match args.direction {
        DirectionArg::Long => Direction::Long,
        DirectionArg::Short => Direction::Short,
    };
    let bars = YahooMarketSource::new()?;
    let frame = crate::application::risk::risk_frame(
        &args.ticker,
        direction,
        args.budget,
        Some(args.stop_mult),
        args.entry,
        &bars,
        Utc::now(),
    )
    .await?;
    Ok(match args.format {
        FormatArg::Table => render_table(&frame),
        FormatArg::Json => render_json(&frame)?,
    })
}

fn render_json(frame: &RiskFrame) -> Result<String, DomainError> {
    #[derive(serde::Serialize)]
    struct Out<'a> {
        frame: &'a RiskFrame,
        framing: &'static str,
        disclaimer: &'static str,
    }
    serde_json::to_string_pretty(&Out {
        frame,
        framing: CALCULATOR_LINE,
        disclaimer: DISCLAIMER,
    })
    .map_err(|e| DomainError::SourceFailure {
        name: "risk".into(),
        message: format!("render failed: {e}"),
    })
}

fn render_table(f: &RiskFrame) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(out, "=== OpenIntel Risk Frame — {} ({:?}) ===", f.ticker, f.direction);
    let _ = writeln!(
        out,
        "generated: {} · bars: {} · ATR(14): {:.2}\n",
        f.generated_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        f.bars_used,
        f.atr
    );
    let _ = writeln!(out, "  entry:          {:>10.2}", f.entry);
    let _ = writeln!(
        out,
        "  stop:           {:>10.2}   ({}×ATR = {:.2}/share)",
        f.stop, f.stop_multiple, f.risk_per_share
    );
    let _ = writeln!(
        out,
        "  size:           {:>10} shares   (notional ${:.2})",
        f.shares, f.notional_usd
    );
    let _ = writeln!(
        out,
        "  max loss:       {:>10.2}   (budget ${:.2})",
        f.max_loss_usd, f.budget_usd
    );
    let _ = writeln!(out, "  1R / 2R / 3R:   {:.2} / {:.2} / {:.2}", f.targets[0], f.targets[1], f.targets[2]);
    if let Some(note) = &f.note {
        let _ = writeln!(out, "\n  note: {note}");
    }
    let _ = writeln!(out, "\n{CALCULATOR_LINE}");
    let _ = writeln!(out, "\n{DISCLAIMER}");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn frame() -> RiskFrame {
        RiskFrame {
            ticker: "NVDA".into(),
            direction: Direction::Long,
            entry: 106.0,
            atr: 4.0,
            stop_multiple: 2.0,
            stop: 98.0,
            risk_per_share: 8.0,
            shares: 25,
            max_loss_usd: 200.0,
            budget_usd: 200.0,
            targets: [114.0, 122.0, 130.0],
            notional_usd: 2650.0,
            bars_used: 16,
            note: None,
            generated_at: chrono::Utc.with_ymd_and_hms(2026, 7, 16, 0, 0, 0).unwrap(),
        }
    }

    #[test]
    fn table_shows_all_numbers_and_framing() {
        let t = render_table(&frame());
        assert!(t.contains("=== OpenIntel Risk Frame — NVDA (Long) ==="));
        assert!(t.contains("stop:                98.00"));
        assert!(t.contains("25 shares"));
        assert!(t.contains("max loss:            200.00"));
        assert!(t.contains("114.00 / 122.00 / 130.00"));
        assert!(t.contains("calculator, not advice"));
        assert!(t.contains("Not financial advice"));
        assert!(!t.contains("note:"));
    }

    #[test]
    fn table_shows_zero_share_note() {
        let mut f = frame();
        f.shares = 0;
        f.max_loss_usd = 0.0;
        f.note = Some("budget too small for one share at this stop distance".into());
        assert!(render_table(&f).contains("note: budget too small"));
    }

    #[test]
    fn json_has_frame_framing_disclaimer() {
        let j = render_json(&frame()).unwrap();
        assert!(j.contains("\"shares\": 25"));
        assert!(j.contains("calculator, not advice"));
        assert!(j.contains("Not financial advice"));
    }
}
```

(If the table alignment assertions fail on exact spacing, loosen to `contains("98.00")`-style checks — pin content, not padding. Note the adaptation.)

Register `pub mod risk;` in `src/cli/mod.rs`.

- [ ] **Step 3: `src/main.rs` arm** (after Pulse):

```rust
        Command::Risk(args) => match openintel::cli::risk::run(&args).await {
            Ok(rendered) => {
                println!("{rendered}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
```

- [ ] **Step 4: Verify, smoke, commit**

Run: `cargo build --all-targets && cargo test && cargo clippy -- -D warnings && cargo fmt`
Run: `cargo run -q -- risk AAPL --budget 200` — expected: a live frame with plausible numbers (Yahoo is keyless/free), exit 0.

```bash
git add src/cli/risk.rs src/cli/args.rs src/cli/mod.rs src/main.rs
git commit -m "feat(cli): openintel risk — ATR stop, budget-capped size, R targets

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 5: MCP — `risk_frame` tool

**Files:**
- Modify: `src/mcp/tools.rs`, `src/mcp/server.rs`

**Interfaces:**
- Consumes: Task 3; the server's existing `market: YahooMarketSource` field (now a `BarSource` — no new field).
- Produces: `tools::{RiskToolArgs, RiskOutput, run_risk_frame}`; `risk_frame` MCP tool.

- [ ] **Step 1: `src/mcp/tools.rs`** — add:

```rust
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum RiskDirectionArg {
    Long,
    Short,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RiskToolArgs {
    /// Ticker symbol, e.g. "NVDA".
    pub ticker: String,
    /// Per-trade risk budget in USD — the most a stop-out may lose.
    pub budget_usd: f64,
    /// Trade direction (default long).
    pub direction: Option<RiskDirectionArg>,
    /// Stop distance in ATR multiples (default 2.0, clamped 0.5-5).
    pub stop_multiple: Option<f64>,
    /// Entry price override (default: last close).
    pub entry: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct RiskOutput {
    pub summary: String,
    pub frame: crate::domain::risk::RiskFrame,
    pub framing: &'static str,
    pub disclaimer: &'static str,
}

pub async fn run_risk_frame(
    args: RiskToolArgs,
    bars: &dyn crate::domain::ports::bar_source::BarSource,
) -> Result<RiskOutput, DomainError> {
    use crate::domain::risk::Direction;
    let direction = match args.direction.unwrap_or(RiskDirectionArg::Long) {
        RiskDirectionArg::Long => Direction::Long,
        RiskDirectionArg::Short => Direction::Short,
    };
    let frame = crate::application::risk::risk_frame(
        &args.ticker,
        direction,
        args.budget_usd,
        args.stop_multiple,
        args.entry,
        bars,
        chrono::Utc::now(),
    )
    .await?;
    let summary = format!(
        "{} {:?} — entry {:.2} · stop {:.2} · {} shares · max loss ${:.2} (≤ ${:.2}) · 1R {:.2}",
        frame.ticker,
        frame.direction,
        frame.entry,
        frame.stop,
        frame.shares,
        frame.max_loss_usd,
        frame.budget_usd,
        frame.targets[0]
    );
    Ok(RiskOutput {
        summary,
        frame,
        framing: "risk_frame is a calculator, not advice — it never recommends taking a trade.",
        disclaimer: DISCLAIMER,
    })
}
```

Test (fake `BarSource`, same fixture bars as application tests; assert summary contains "25 shares" and framing present).

- [ ] **Step 2: `src/mcp/server.rs`** — tool in the `#[tool_router]` block (description verbatim from the spec):

```rust
    #[tool(
        description = "Deterministic risk calculator: given a ticker, a per-trade risk budget in \
                       USD, and a direction, returns an ATR(14)-based stop level, the whole-share \
                       size that caps a stop-out at the budget, max loss, and 1R/2R/3R reference \
                       levels. It does NOT recommend trades — combine it with analyze_ticker / \
                       x_pulse, present the numbers to the user, and get their explicit approval \
                       before any execution step. Read-only — does not trade."
    )]
    async fn risk_frame(
        &self,
        Parameters(args): Parameters<tools::RiskToolArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = tools::run_risk_frame(args, &self.market)
            .await
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let json = serde_json::to_string_pretty(&out)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![ContentBlock::text(json)]))
    }
```

(`&self.market` coerces to `&dyn BarSource` — `YahooMarketSource` implements it as of Task 2.)

- [ ] **Step 3: Verify + commit**

```bash
git add src/mcp/tools.rs src/mcp/server.rs
git commit -m "feat(mcp): risk_frame tool — approval-gated calculator contract

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 6: Docs

**Files:**
- Modify: `README.md`

- [ ] **Step 1:** After the X Pulse section add:

```markdown
## Risk framing (calculator, not advice)

Turn a trade idea into exact numbers: `openintel risk NVDA --budget 200` returns an ATR(14)-based stop, the whole-share size that caps a stop-out at your budget, max loss, and 1R/2R/3R reference levels. Deterministic math over free Yahoo daily bars — it never recommends taking a trade. Also exposed to agents as the `risk_frame` MCP tool, whose contract requires presenting the numbers and getting your explicit approval before any execution step.
```

Also add `risk_frame` to the MCP tools table (row: `risk_frame` | ATR stop + budget-capped size + R targets for one trade idea).

- [ ] **Step 2: Verify + commit**

Run: `cargo test 2>&1 | tail -3`

```bash
git add README.md
git commit -m "docs: risk framing section + MCP tool row

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```
