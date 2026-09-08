# OpenIntel

OpenIntel is a security-first Rust CLI and MCP server that turns market and social data into evidence-gated trade analysis. It is the intelligence layer in a three-part loop: the user's AI agent is the brain, a broker's own MCP (Robinhood Agentic Trading) is the execution rail, and OpenIntel informs — it never trades.

## What we can never compromise on

### 1. The read-only boundary

OpenIntel never places a trade, never touches a broker, never holds broker credentials. Execution happens only through the broker's MCP, gated by the broker's approvals. That boundary is the entire safety model. Every tool we expose stays read-only (writing our own local journal is fine); anything that would blur the line into execution belongs in the broker's product, not here.

### 2. Evidence honesty

The codebase's signature move is refusing to know things it doesn't know. Gates are three-valued (`Pass` / `Fail` / `Unknown`) and unverifiable evidence fails closed — an `Unknown` caps the verdict, never upgrades it. Reviews refuse conclusions below a meaningful sample (n ≥ 30). Missing data becomes a note in the output, never a silent zero. New features inherit this posture: when you can't verify, say so in the result and degrade the claim.

### 3. Calculator, never advisor

Our copy computes; it does not predict or recommend. `high_confidence` means conformance to a setup template, never probability of profit. Scores are labeled unvalidated until the review loop validates them. Watch for advisory creep in strings, docs, and MCP tool descriptions — "consider buying" is a defect the compiler can't catch. MCP descriptions that precede spending or execution must spell out cost and require user confirmation (see `x_pulse` and `risk_frame` for the pattern).

### 4. Hexagonal purity

The domain is pure and synchronous: no IO, no clock, no filesystem, no `Utc::now()`. Time and paths are stamped at the application edge and injected. Adapters implement port traits; construction happens only at the two composition roots — `main.rs` and `mcp::server::serve()`. A new source is a new adapter plus wiring at both roots, with zero engine or application changes. Only `main.rs` and the `cli/` renderers print.

## A note from Cloud

I love to build, and I focus on building complex things as simple as possible. Channel yagni. Don't preserve complexity because it exists, and don't add machinery because it looks impressive — find the real constraint and fight for the smallest model that makes correct behavior unsurprising. Bold ideas are welcome when they meaningfully help. The code should tell the story without comments; comment only what the code can't say, and keep comments in sync when behavior changes. Tests are focused, not slop — no endless smoke tests.

## Glossary

- **symbol** — Yahoo's form, validated by `Ticker::parse`, carrying its `AssetClass`: `AAPL` (equity), `BTC-USD` or bare `BTC` (crypto), `EURUSD=X` (forex), `ES=F` (future). Every surface accepts all four; the clock, sizing, and gates branch on the class, and a gate with no evidence source for a class reports `unknown`.
- **verdict** — the tiered dip outcome: `no_setup` / `watch` / `high_confidence`.
- **gate** — one three-valued check (`GateStatus`) feeding a verdict.
- **session** — `Intraday` or `PostClose`; intraday runs cap at `watch` because the day bar isn't final.
- **frame** — a deterministic per-trade calculation (`RiskFrame`, `MarginFrame`, `OptionFrame`): exact numbers, no opinion.
- **journal** — an append-only JSONL log under `~/.openintel/`, graded later against forward returns.
- **pulse** — the paid, opt-in X influencer catalyst feed. Every read costs real money.
- **clock** — the NYSE session state for an injected instant: `pre_market` / `open` / `post_close` / `closed` (with the reason) / `unknown` outside the vendored calendar. `market_clock` is the tool; agents call it first so stale data is visible.
- **brief** — the day's dated evidence with no ticker: clock, vendored macro releases, the earnings calendar, overnight filings and headlines for the tickers passed, last chatter counts. Evidence, never proposals.
- **composition roots** — `main.rs` and `mcp::server::serve()`, the only places adapters are constructed.

## The ways to hurt yourself

1. **Spending real money.** The `#[ignore]`d X tests and any `pulse` run bill a real API (~$0.05 minimum per call). Run them only when the developer explicitly asks.
2. **Touching real local state.** `~/.openintel/` holds the developer's real journals and the OS keychain holds real credentials. Tests write to temp dirs (see the existing journal tests) and leave the keychain to the `#[ignore]`d test that exists for it.
3. **Trusting free endpoints as stable.** Yahoo's and Nasdaq's endpoints are keyless and unofficial (Nasdaq also refuses non-browser user agents); their failure mode must stay a clean error with a note, never a panic or a fabricated value. SEC EDGAR is official — keep the `OPENINTEL_SEC_CONTACT` identification honored.
4. **Letting the vendored calendars lapse.** The NYSE holiday table in `domain/clock.rs` (2026–2027) and the macro release calendar in `domain/macro_calendar_2026.json` (2026) are data with coverage windows. Past coverage the answer is `unknown`: honest, and useless. Each December, refresh both from the exchange and agency schedules and move the coverage window and its tests with them.

## Feature workflow

Every new feature, spec, or design starts as a GitHub issue. Then comes a plan, presented to the developer as an HTML-communicated document for approval — implementation starts only after a pick. There is no in-repo spec/plan directory (a former `docs/superpowers/` convention was deliberately retired; its file pattern is history, not instruction — recreate nothing there).

## Hit every surface

A feature that works in one surface and is missing elsewhere is the most likely defect here. Before calling work done, check:

- **CLI and MCP.** Most capabilities ship as both a subcommand and an MCP tool. Adding one without deciding about the other is half a feature — and the decision may legitimately be "CLI only".
- **Both composition roots.** An adapter wired into `main.rs` but not `mcp::server::serve()` (the `analyze` dip-signal asymmetry is the cautionary tale) ships two different products.
- **README.** The usage tables and MCP tool table are the product's documentation; they describe shipped reality, never aspiration.
- **Secrets resolution.** Anything credentialed resolves env-first, then keychain, wrapped in `SecretString`. Plaintext never touches disk or logs.

## Verifying

- `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check` — all three green before any PR.
- `#[ignore]`d tests hit live networks (Yahoo, EDGAR, Reddit, Bluesky, X, keychain) and are excluded by default. Run a specific one only on request, and never the paid X one uninvited.

## Pull requests

- Never open a PR unless the developer asks. Real PRs, not drafts — drafts skip review-bot coverage. Rebase onto latest `main` first.
- Conventional commit titles in plain language: `feat(dip): gated dip-setup scanner`.
- Body: the problem in a sentence or two, then how it was solved. End with the model and harness that did the work.
- One concern per PR. If the description says "also", split it.
