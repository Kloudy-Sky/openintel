<p align="center">
  <h1 align="center">🎯 OpenIntel</h1>
</p>

<p align="center">
  <em>A structured intelligence engine with hybrid semantic search, strategy detection, and trade journaling — built in Rust.</em>
</p>

<p align="center">
  <a href="https://github.com/Kloudy-Sky/openintel/actions"><img src="https://github.com/Kloudy-Sky/openintel/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/Kloudy-Sky/openintel/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue" alt="License"></a>
  <a href="https://github.com/Kloudy-Sky/openintel"><img src="https://img.shields.io/badge/rust-1.75%2B-orange" alt="Rust"></a>
</p>

---

> Most vector databases want you to deploy Kubernetes, provision cloud infra, and pay per query. OpenIntel is a single binary and a single `.db` file. Add intelligence, search it with keywords or vectors, detect trading signals, and journal your trades. Copy two files to a new machine and you're done. If that sounds too simple, it is — that's the point.

---

## Highlights

- **Hybrid search** — BM25 keyword matching + semantic vector similarity with Reciprocal Rank Fusion
- **Strategy engine** — pluggable signal detection with built-in earnings momentum, tag convergence, and cross-intel convergence strategies
- **Opportunity scoring** — confidence × edge × √liquidity, ranked and ready to trade
- **Trade journal** — track entries, exits, P&L, and auto-resolve trades against external sources
- **Alert system** — volume spikes, confidence decay, actionable item tracking
- **Daily summaries** — category breakdown, trending tags, confidence distribution
- **SQLite everything** — single file, zero infrastructure, portable across machines
- **Pluggable embeddings** — Voyage AI, OpenAI, or none (keyword search still works)

## Installation

Build from source (requires Rust 1.75+):

```bash
git clone https://github.com/Kloudy-Sky/openintel.git
cd openintel
cargo install --path .
```

Or grab the release binary:

```bash
cargo build --release
# → target/release/openintel
```

## Quick Start

```console
$ openintel add market '{"title":"AAPL beats earnings","body":"Revenue up 8% YoY, services at ATH","tags":["AAPL","earnings","beat"],"confidence":0.9}'

$ openintel search "Apple revenue"

$ openintel opportunities --hours 48

$ openintel scan --hours 24

$ openintel stats
```

## Commands

| Command | Description |
|---------|-------------|
| `add <category> '<json>'` | Add an intel entry |
| `search <query>` | BM25 keyword search |
| `semantic <query>` | Vector similarity search |
| `think <query>` | Hybrid search (BM25 + vector + RRF) |
| `query <category>` | Query by category with filters |
| `opportunities` | Run all strategies, rank signals |
| `scan` | Alert scan — volume spikes, decay, actionable items |
| `summarize` | Daily intelligence summary |
| `pending` | Show actionable items needing attention |
| `stats` | Database statistics |
| `tags [category]` | Tag frequency counts |
| `trade-add '<json>'` | Open a trade |
| `trade-resolve <id> <outcome> <pnl>` | Close a trade |
| `trades` | List trades with filters |
| `reindex` | Re-embed entries missing vectors |
| `export` | Export entries as JSON |

## Strategies

OpenIntel ships with three detection strategies. Each implements the `Strategy` trait and can be extended:

| Strategy | Signal | What it detects |
|----------|--------|-----------------|
| `earnings_momentum` | Tag frequency + sentiment | Stocks with multiple bullish/bearish mentions across sources |
| `tag_convergence` | Co-occurring tags | Tags appearing together repeatedly, suggesting a trend |
| `convergence` | Cross-source clustering | Same topic from multiple source types with time-decay weighted sentiment |

```console
$ openintel opportunities --hours 48
{
  "strategies_run": 3,
  "entries_scanned": 59,
  "opportunities": [
    {
      "title": "CRCL — bullish earnings momentum (4 signals)",
      "confidence": 0.80,
      "score": 80,
      "suggested_direction": "bullish",
      "market_ticker": "CRCL",
      "strategy": "earnings_momentum"
    }
  ]
}
```

### Custom Strategies

Implement `domain::ports::strategy::Strategy` to add your own:

```rust
pub trait Strategy: Send + Sync {
    fn name(&self) -> &str;
    fn detect(&self, ctx: &DetectionContext) -> Vec<Opportunity>;
}
```

See [src/application/strategies/](src/application/strategies/) for examples.

## Architecture

```
domain/           Pure types, zero dependencies
  entities/       IntelEntry, Trade
  values/         Category, Confidence, Decay
  ports/          Repository, Embedding, Strategy traits

application/      Use-case orchestration
  strategies/     EarningsMomentum, TagConvergence, Convergence

infrastructure/   Adapters
  sqlite/         Persistence (rusqlite)
  embeddings/     Voyage AI, OpenAI, NoOp

cli/              Commands and argument parsing
```

Hexagonal architecture — domain logic knows nothing about databases, APIs, or the CLI.

## Embedding Providers

Configure via environment variables:

```bash
# Voyage AI (recommended)
export OPENINTEL_EMBEDDING_PROVIDER=voyage
export OPENINTEL_EMBEDDING_MODEL=voyage-3-lite
export VOYAGE_API_KEY=pa-xxx

# OpenAI
export OPENINTEL_EMBEDDING_PROVIDER=openai
export OPENINTEL_EMBEDDING_MODEL=text-embedding-3-small
export OPENAI_API_KEY=sk-xxx

# No embeddings (keyword search only)
# Just don't set the provider — everything else still works.
```

## Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `OPENINTEL_DB` | `./openintel.db` | Database path |
| `OPENINTEL_EMBEDDING_PROVIDER` | `noop` | `voyage`, `openai`, or `noop` |
| `OPENINTEL_EMBEDDING_MODEL` | provider default | Embedding model name |
| `VOYAGE_API_KEY` | — | Voyage AI key |
| `OPENAI_API_KEY` | — | OpenAI key |

## Categories

Intel entries are typed by category:

`market` · `newsletter` · `social` · `trading` · `opportunity` · `competitor` · `general` · `earnings` · `macro` · `crypto` · `weather` · `politics` · `technology` · `research` · `regulatory` · `sentiment` · `geopolitical` · `sector` · `company`

## Use Cases

- **Autonomous agents** — structured memory and retrieval
- **Trading systems** — signal detection → opportunity scoring → trade journaling
- **Research pipelines** — collect, tag, search, and surface insights
- **Newsletter analysis** — archive and semantically query content
- **Competitive intelligence** — track moves with confidence and decay
- **Personal knowledge base** — your embedded second brain

## Development

```bash
cargo test           # Run tests
cargo fmt            # Format
cargo clippy         # Lint
cargo build --release  # Optimized build
RUST_LOG=debug cargo run -- stats  # Debug logging
```

## Contributing

1. Fork → branch (`feat/my-feature`) → tests → `cargo fmt` → `cargo clippy` → PR
2. All PRs run CI (fmt, clippy, tests) and automated Claude Code Review

## License

MIT — see [Cargo.toml](Cargo.toml).

---

<p align="center">
  Built with 🎩 by <a href="https://github.com/jrvsai">Jarvis</a> at <a href="https://github.com/Kloudy-Sky">Kloudy-Sky</a>
</p>
