# Risk Frame — Deterministic Stop/Size Calculator — Design

**Date:** 2026-07-16
**Status:** Draft — awaiting user review

## Goal

Turn "I want to take this trade with $200 of risk" into exact, deterministic
numbers: an ATR-based stop level, the share size that makes a stop-out lose
exactly the budget, and 1R/2R/3R target references. **A calculator, not an
advisor** — it never says "buy"; it says *"if you go long here with this stop,
this size caps your loss at $200."* This is step 2 of the trading-loop
sequence (analyze/pulse → **risk_frame** → playbook + broker MCP): the agent
composes it with `analyze_ticker` and `x_pulse`; the human approves; execution
comes later and elsewhere. openintel stays analysis-only.

## The math (pinned)

- **Bars:** last ~3 months of daily OHLC (the same Yahoo v8 request the
  snapshot already makes — the parser just starts keeping `high`/`low`).
- **True Range:** `TR_i = max(high_i − low_i, |high_i − prev_close|, |low_i − prev_close|)`.
- **ATR(14):** simple mean of the last 14 TRs (SMA, not Wilder smoothing —
  simpler, deterministic, documented; revisit only with evidence it matters).
  Requires ≥ 15 bars, else `SourceFailure("not enough history for ATR(14)")`.
- **Entry reference:** last close, overridable via `entry` param (> 0, finite).
- **Stop:** long → `entry − k·ATR`; short → `entry + k·ATR`. `k` =
  `stop_multiple`, default `2.0`, clamped `0.5..=5.0`. A long stop that would
  be ≤ 0 errors ("stop below zero — widen budget or lower multiple").
- **Risk per share:** `k·ATR`. **Shares:** `floor(budget / risk_per_share)`
  (whole shares v1). `shares == 0` is a valid, explicit outcome: the frame
  still returns, with a note that the budget can't buy one share at this stop
  distance.
- **Max loss:** `shares × risk_per_share` (≤ budget by construction — the
  actual capped number, not the requested budget).
- **Targets:** `1R/2R/3R` price levels = `entry ± n × risk_per_share`
  (direction-signed) — reference exits, not predictions.
- **Notional:** `shares × entry` (so the human sees capital-at-work, not just
  risk).

## Architecture

- `domain/values/bar.rs` (new): `pub struct Bar { pub high: f64, pub low: f64, pub close: f64 }`.
- `domain/risk.rs` (new, pure): `true_ranges(&[Bar]) -> Vec<f64>`,
  `atr(&[Bar], period: usize) -> Option<f64>`, and
  `frame(bars, direction, entry, budget_usd, stop_multiple) -> Result<RiskFrame, DomainError>`
  — all synchronous, no clock, no IO. `Direction { Long, Short }` (serde
  lowercase). `RiskFrame { ticker: String, direction, entry, atr, stop_multiple,
  stop, risk_per_share, shares: u64, max_loss_usd, budget_usd, targets: [f64; 3],
  notional_usd, bars_used: usize, generated_at }` — `Serialize`.
- `domain/ports/bar_source.rs` (new): `#[async_trait] pub trait BarSource:
  Send + Sync { async fn bars(&self, ticker: &Ticker) -> Result<Vec<Bar>, DomainError>; }`
  — a separate small port so `MarketDataSource`/`MarketSnapshot` and every
  existing mock stay untouched.
- Yahoo adapter: `Quote` gains `high`/`low` arrays (`#[serde(default)]`);
  a pure `parse_bars` zips h/l/c rows (skipping rows with any missing leg);
  `impl BarSource for YahooMarketSource` reuses the existing chart request.
- `application/risk.rs` (new): validate ticker + params (budget finite > 0;
  entry finite > 0 when given), clamp `stop_multiple`, fetch bars, default
  entry to last close, call `domain::risk::frame`, stamp `generated_at`
  (clock at the edge).
- CLI: `openintel risk NVDA --budget 200 [--direction long] [--stop-mult 2]
  [--entry 207.4] [--format table|json]` — leaf `src/cli/risk.rs` returning
  Strings (main prints), table shows every number plus the calculator framing
  line and the standard disclaimer.
- MCP: `risk_frame` tool. Description (the contract, verbatim):

  > Deterministic risk calculator: given a ticker, a per-trade risk budget in
  > USD, and a direction, returns an ATR(14)-based stop level, the whole-share
  > size that caps a stop-out at the budget, max loss, and 1R/2R/3R reference
  > levels. It does NOT recommend trades — combine it with analyze_ticker /
  > x_pulse, present the numbers to the user, and get their explicit approval
  > before any execution step. Read-only — does not trade.

## Errors

Insufficient history, flat/degenerate bars (ATR = 0 → error, not divide-by-
zero), non-positive budget/entry, stop ≤ 0 — all `SourceFailure`-style clear
messages. Bar-fetch failures surface like every other market failure.

## Testing (hermetic)

- Pure math vectors: hand-computed TR/ATR on a small fixture (incl. a gap day
  where `|high − prev_close|` dominates); long + short stop/size/targets;
  `floor` sizing edge (budget just under 1 share → `shares 0` + note; exactly
  1 share); `max_loss ≤ budget` invariant; clamps (k 0.1→0.5, 9→5).
- `parse_bars`: happy path, skips rows with missing legs, empty.
- Application: param validation, entry default = last close, mock `BarSource`.
- Render: table contains stop/size/max-loss/targets/disclaimer; JSON round-trip.
- One `#[ignore]`d live test (Yahoo bars, keyless — free).

## Non-goals (YAGNI)

- No options structures (needs chain data — Robinhood MCP, playbook phase).
- No fractional shares (v1 floors; revisit when the playbook targets Robinhood
  fractional orders).
- No account-level caps (daily loss, exposure) — those are playbook policy,
  not per-trade math.
- No swing-low/high or support/resistance stop methods — ATR multiple only.
- No Wilder smoothing, no configurable ATR period.

## Files

**Create:** `src/domain/values/bar.rs`, `src/domain/risk.rs`,
`src/domain/ports/bar_source.rs`, `src/application/risk.rs`, `src/cli/risk.rs`
**Modify:** domain/application/cli mod registrations, `src/cli/args.rs`
(`Risk(RiskArgs)`), `src/adapters/market/yahoo/{mod.rs,response.rs}`,
`src/main.rs`, `src/mcp/{server.rs,tools.rs}`, `README.md`
