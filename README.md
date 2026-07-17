# OpenIntel

Security-first CLI that fuses social-media chatter with market action into a **speculation report** — a crowding & divergence detector for a ticker.

> **Not financial advice.** OpenIntel is a research/screening tool. Social data is noisy and easily manipulated. Do your own diligence.

## Quickstart

Run it immediately — no install; market data comes from Yahoo Finance; Reddit and Bluesky sentiment go live once configured (see below):

```bash
cargo run -- analyze AAPL
```

Or install it on your PATH for the shorter `openintel` command used throughout this README:

```bash
cargo install --path .
openintel analyze AAPL
```

> **Market data is live (Yahoo Finance, keyless). Reddit and Bluesky are live when configured (see below) — and see *X Pulse* below for paid catalyst tracking.** `analyze` fetches over the network — offline or unconfigured sources degrade gracefully with a note.

## Usage

```bash
# All social sources + market snapshot (default)
openintel analyze AAPL

# Narrow to specific sources
openintel analyze AAPL --enable-reddit --enable-bluesky

# Social only, JSON output
openintel analyze AAPL --no-market --format json
```

| Flag | Meaning |
|---|---|
| `--enable-reddit/--enable-bluesky` | Restrict to these sources (none given → all enabled) |
| `--no-market` | Skip the market snapshot (social-only report) |
| `--limit <N>` | Posts per source (default 50) |
| `--format table\|json` | Output format (default table) |

## Enable the Reddit source (optional)

Run `openintel setup reddit` — it walks you through creating a free script app, verifies your credentials live, and saves them to your OS keychain. Rotate by re-running it; remove with `openintel setup reddit --forget`.

<details>
<summary>CI / power users: environment variables instead</summary>

Env vars always override the keychain. Create a **script** app at <https://www.reddit.com/prefs/apps>, then:

```bash
export OPENINTEL_REDDIT_CLIENT_ID=your_client_id
export OPENINTEL_REDDIT_CLIENT_SECRET=your_secret
openintel setup reddit   # non-interactive when piped; verifies from env
```

</details>

## Enable the Bluesky source (optional)

Run `openintel setup bluesky` — it walks you through creating a free app password, verifies your credentials live, and saves them to your OS keychain. Rotate by re-running it; remove with `openintel setup bluesky --forget`.

<details>
<summary>CI / power users: environment variables instead</summary>

Env vars always override the keychain. Get your handle and create an app password at <https://bsky.app/settings/app-passwords>, then:

```bash
export OPENINTEL_BLUESKY_HANDLE=yourname.bsky.social
export OPENINTEL_BLUESKY_APP_PASSWORD=xxxx-xxxx-xxxx-xxxx
openintel setup bluesky   # non-interactive when piped; verifies from env
```

</details>

## X Pulse (optional — paid X API)

Catalyst posts from specific high-impact X accounts (a POTUS tariff post, a CEO
announcement) — surfaced as **events to reason about**, never averaged into
sentiment. X's API is pay-per-use (~$0.005 per post read, deduped over 24h), so
the pulse is strictly opt-in: nothing calls X unless you run it.

```bash
openintel setup x                                                       # guided token setup (verify reads ≈ $0.05)
openintel pulse TSLA --accounts elonmusk --keywords tesla,robotaxi     # ≤ 20 reads ≈ $0.10 max
```

Add `--keywords` with the company's own vocabulary — influencer posts say "Tesla", not "$TSLA".

Note: X's API bills a minimum of 10 post reads per call, so even `--limit 1` costs ≈ $0.05.

No `--accounts` → a small macro default list (POTUS, White House, Musk, the Fed).
Via MCP, the `x_pulse` tool asks the agent to research which accounts matter for
your ticker and confirm the cost with you before spending.

## Risk framing (calculator, not advice)

Turn a trade idea into exact numbers: `openintel risk NVDA --budget 200` returns an ATR(14)-based stop, the whole-share size that caps a stop-out at your budget, max loss, and 1R/2R/3R reference levels. Deterministic math over free Yahoo daily bars — it never recommends taking a trade. Also exposed to agents as the `risk_frame` MCP tool, whose contract requires presenting the numbers and getting your explicit approval before any execution step. Run intraday, the entry default is the live price and ATR includes today's still-forming bar — re-run near the close for settled numbers.

## Use with an AI agent (MCP)

OpenIntel can run as a local **MCP server** so an AI agent can consult its analysis while
you trade through **Robinhood's official Agentic Trading MCP**. OpenIntel is the
intelligence layer; the agent is the brain; Robinhood's MCP is execution.

```text
your agent (Claude Code on your subscription / ChatGPT / Codex / Cursor / Grok)
  ├─ MCP → openintel                          (analysis — this tool)
  └─ MCP → agent.robinhood.com/mcp/trading    (execution, sandboxed agentic wallet)
```

Wire up both MCPs (Claude Code shown; other agents add the same commands in their MCP
settings). Requires `openintel` on your PATH — see [Quickstart](#quickstart):

```bash
claude mcp add openintel -- openintel mcp
claude mcp add robinhood-trading --transport http https://agent.robinhood.com/mcp/trading
```

Tools exposed (all **read-only** — OpenIntel never places trades):

| Tool | What it does |
|---|---|
| `analyze_ticker` | One symbol → full speculation report (sentiment, speculation index, crowding, alignment) |
| `scan_watchlist` | A list of symbols → reports, run concurrently |
| `compare_tickers` | Rank a set by `crowding` / `speculation_index` / `net_sentiment` / `divergence` |
| `list_sources` | Which data sources are available |
| `risk_frame` | ATR stop + budget-capped size + R targets for one trade idea |

### ⚠️ Risk & responsibility — read before connecting a broker

Connecting an AI agent to a brokerage MCP means **an AI can place real trades with real money
in your account.** Understand exactly what you're authorizing:

- **OpenIntel is a screener, not advice — and not a proven edge.** It surfaces *attention* and
  *crowding / divergence* signals from social chatter. Social sentiment is noisy, easily
  manipulated (bots, coordinated pumps), and mostly coincident-to-lagging — not predictive.
  Treat its output as one input to your own judgment, never as a buy/sell instruction.
- **AI agents make mistakes.** They hallucinate, misread data, act on stale or incomplete
  information, and can behave unexpectedly — including placing a wrong or oversized trade.
  Trading automatically on automated signals can lose money quickly.
- **You are fully responsible for every trade placed.** This software has no warranty and is
  not financial advice. Nothing here is a strategy shown to be profitable.
- **Only fund money you can afford to lose — entirely.** Use a dedicated broker *agentic
  sub-account* and fund a deliberately small wallet. **That balance is your hard blast-radius
  cap** — the agent cannot spend beyond it.
- **Keep the broker's approval-required mode on.** Review and approve trades before they
  execute; do not authorize unattended / autonomous trading until you genuinely trust the
  setup. Connecting also grants the agent broad **read** access to your accounts — a privacy
  surface.
- **Scope / status:** Robinhood's Agentic Trading is a **beta, US-only, equities-only**
  product. OpenIntel itself is early software (live market data via Yahoo; Reddit and Bluesky sentiment live when configured); the intelligence layer is meant to be iterated on.

By design, **OpenIntel never executes trades, touches a broker, or holds credentials** —
execution happens only through the broker's own MCP, gated by the broker's controls and your
approval. That boundary *is* the safety model; keep it.

## What it computes

- **net sentiment** — mean per-post polarity `[-1, 1]`
- **speculation index** — share of posts using options/leverage jargon
- **rvol / pct change** — volume vs average, day move
- **crowding** — blended speculation + RVOL + IV rank `[0, 1]`
- **alignment** — `ConfirmingBullish/Bearish`, `Diverging`, or `Quiet`

## Architecture

Hexagonal (ports & adapters). The domain is pure and synchronous; IO and the clock live at the edge.

- `domain/` — entities, value objects, the pure `SpeculationEngine`, and port traits.
- `adapters/` — `LexiconAnalyzer`, the `YahooMarketSource` (real, keyless), the `RedditSource` and `BlueskySource` (real, credential-gated — no mock sources).
- `config/` — secrets resolution (env + OS keychain, via `secrecy`) and runtime settings.
- `cli/` — clap args, orchestration, rendering.

Secrets come from environment variables (`OPENINTEL_REDDIT_CLIENT_ID`, `OPENINTEL_REDDIT_CLIENT_SECRET`, `OPENINTEL_BLUESKY_HANDLE`, `OPENINTEL_BLUESKY_APP_PASSWORD`, `OPENINTEL_MARKET_API_KEY`) or the OS keychain (written only by `openintel setup` after a live verify; env always wins), wrapped in `SecretString` — plaintext never touches disk, never logged.

## Extending

**Add a social source** (e.g. real Bluesky):
1. New struct in `src/adapters/sources/`, `impl SocialDataSource`.
2. Add a `SourceKind` variant in `src/domain/values/source_kind.rs` if new.
3. Construct it at the composition roots — `main.rs` (analyze branch) and `mcp::server::serve()` — and push it onto the injected social list. No engine or application change.

**Add a market source** (e.g. a keyed provider):
1. New struct in `src/adapters/market/`, `impl MarketDataSource`.
2. Construct it at the composition roots — `main.rs` (analyze branch) and `mcp::server::serve()` — and it flows in through the injected `&dyn MarketDataSource`. No engine or application change.

**Swap the analyzer** (lexicon → LLM/ML):
1. New struct in `src/adapters/analyzer/`, `impl PostAnalyzer`. No engine change.

## Development

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```
