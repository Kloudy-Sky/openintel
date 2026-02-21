# OpenIntel

**Structured intelligence knowledge base with hybrid semantic + keyword search, built in Rust.**

OpenIntel is an embedded, file-based intelligence store built on SQLite with optional vector embeddings. It's designed for autonomous agents, trading systems, research pipelines, and anyone who needs structured signal storage with powerful retrieval.

## Why OpenIntel?

Most vector databases are designed for large-scale cloud deployments with complex infrastructure. OpenIntel takes the opposite approach — **SQLite-simple, single-file, zero infrastructure.** Think of it as "SQLite for intelligence."

- **Structured entries** with categories, tags, confidence scores, and metadata
- **Hybrid search** — BM25 keyword search + semantic vector similarity with Reciprocal Rank Fusion (RRF)
- **Trade tracking** — built-in trade journal with P&L resolution
- **Embedded** — single `.db` file, no server, no Docker, no K8s
- **Pluggable embeddings** — bring your own embedding provider (Voyage, OpenAI, or none)
- **Fast** — Rust-native performance with compile-time safety
- **Portable** — just copy the binary and your `.db` file

## Architecture

OpenIntel follows **Domain-Driven Design (DDD)** principles with **Hexagonal Architecture**:

```
┌──────────────────────────────────────────────┐
│                CLI Layer                      │
│  (main.rs, cli/commands.rs)                  │
├──────────────────────────────────────────────┤
│          Application Layer                    │
│  Use Cases:                                  │
│  • add_intel  • search  • query              │
│  • stats      • trade   • reindex            │
├──────────────────────────────────────────────┤
│            Domain Layer                       │
│  Entities:                                   │
│    • IntelEntry  • Trade                     │
│  Values:                                     │
│    • Category  • Confidence  • TradeOutcome  │
│  Ports (interfaces):                         │
│    • IntelRepository  • TradeRepository      │
│    • EmbeddingPort    • VectorStore          │
├──────────────────────────────────────────────┤
│        Infrastructure Layer                   │
│  Adapters:                                   │
│    • SQLite (rusqlite) - persistence         │
│    • Voyage AI - embeddings                  │
│    • OpenAI - embeddings                     │
│    • NoOp - no embeddings                    │
└──────────────────────────────────────────────┘
```

**Why this matters:**
- Domain logic is isolated from databases and APIs
- Easy to swap SQLite for Postgres or S3
- Easy to add new embedding providers
- Testable without infrastructure dependencies

## Quick Start

### Install from crates.io (coming soon)

```bash
cargo install openintel
```

### Building from source

```bash
# Clone the repo
git clone https://github.com/Kloudy-Sky/openintel.git
cd openintel

# Build release binary
cargo build --release

# Binary will be at target/release/openintel
./target/release/openintel --help

# Or install to ~/.cargo/bin
cargo install --path .
```

### First usage

```bash
# Add an intel entry
openintel add market '{"title":"AAPL earnings beat","body":"Revenue up 8% YoY","tags":["AAPL","earnings"],"confidence":0.9}'

# Keyword search
openintel search "Apple revenue"

# Query by category
openintel query market --limit 10

# Stats
openintel stats
```

## CLI Commands

### Intel Management

```bash
# Add an entry
openintel add <category> '<json>'

# Categories: market, newsletter, social, trading, opportunity, competitor, general
# JSON fields: title (required), body, source, tags, confidence, actionable, metadata

# Example
openintel add market '{
  "title": "Fed signals dovish pivot",
  "body": "FOMC minutes suggest 25bp cut likely in March...",
  "tags": ["fed", "rates", "macro"],
  "confidence": 0.8,
  "actionable": true,
  "source": "Reuters"
}'
```

### Search & Query

```bash
# Keyword search (BM25)
openintel search "federal reserve rates" --limit 10

# Semantic vector search (requires embedding provider)
openintel semantic "monetary policy changes" --limit 10

# Hybrid search (keyword + semantic with RRF fusion)
openintel think "interest rate policy" --limit 10

# Query by category
openintel query market --limit 20 --since 2024-01-01 --tag fed
```

### Analytics

```bash
# Database statistics
openintel stats

# List tags with counts
openintel tags

# List tags for a specific category
openintel tags market
```

### Trade Journal

```bash
# Add a trade
openintel trade-add '{
  "ticker": "AAPL",
  "direction": "long",
  "contracts": 100,
  "entry_price": 185.50,
  "thesis": "Earnings momentum + services growth"
}'

# Resolve a trade
openintel trade-resolve <trade-id> win 350 --exit-price 189.00

# List trades
openintel trades --limit 20
openintel trades --since 2024-01-01
openintel trades --resolved true
```

### Export & Maintenance

```bash
# Export entries as JSON
openintel export --since 2024-01-01 --category market > export.json

# Re-embed entries missing vectors (after adding embedding provider)
openintel reindex
```

## Hybrid Search Architecture

OpenIntel combines **BM25 keyword matching** with **vector semantic similarity** using **Reciprocal Rank Fusion (RRF)**:

1. **BM25 Search**: SQLite full-text search on title + body + tags
2. **Vector Search**: Cosine similarity on embeddings (if provider configured)
3. **RRF Fusion**: Combines rankings from both methods for optimal results

**Why RRF?**
- Keyword search finds exact term matches
- Vector search finds semantic/conceptual matches
- RRF merges both without score normalization issues
- Works even if only one method returns results

## Embedding Providers

OpenIntel supports pluggable embedding providers. Configure via environment variables:

### Voyage AI (recommended)

```bash
export OPENINTEL_EMBEDDING_PROVIDER=voyage
export OPENINTEL_EMBEDDING_MODEL=voyage-3-lite  # or voyage-4-lite
export VOYAGE_API_KEY=pa-xxx...
```

### OpenAI

```bash
export OPENINTEL_EMBEDDING_PROVIDER=openai
export OPENINTEL_EMBEDDING_MODEL=text-embedding-3-small
export OPENAI_API_KEY=sk-xxx...
```

### No Embeddings (keyword-only)

```bash
# Don't set OPENINTEL_EMBEDDING_PROVIDER
# or set it to "noop"
```

Without an embedding provider, you still get fast keyword search — you just lose semantic/hybrid search capabilities.

## Database Schema

The database is plain SQLite — you can query it directly with `sqlite3`:

### Intel Entries

```sql
CREATE TABLE intel (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  category TEXT NOT NULL,
  title TEXT NOT NULL,
  body TEXT,
  source TEXT,
  tags TEXT,  -- JSON array
  confidence REAL,
  actionable INTEGER,  -- 0 or 1
  metadata TEXT,  -- JSON object
  created_at TEXT NOT NULL,
  expires_at TEXT,
  embedding_text TEXT,  -- cached text for embedding
  embedding BLOB  -- vector bytes
);
```

### Trade Journal

```sql
CREATE TABLE trades (
  id TEXT PRIMARY KEY,  -- UUID
  ticker TEXT,
  series_ticker TEXT,
  direction TEXT,  -- long/short/yes/no
  contracts INTEGER,
  entry_price REAL,
  exit_price REAL,
  thesis TEXT,
  outcome TEXT,  -- win/loss/scratch
  pnl_cents INTEGER,
  resolved_at TEXT,
  created_at TEXT NOT NULL
);
```

## Configuration

OpenIntel looks for configuration in environment variables:

| Variable | Default | Description |
|----------|---------|-------------|
| `OPENINTEL_DB_PATH` | `./openintel.db` | SQLite database file path |
| `OPENINTEL_EMBEDDING_PROVIDER` | `noop` | `voyage`, `openai`, or `noop` |
| `OPENINTEL_EMBEDDING_MODEL` | provider default | Model name (e.g., `voyage-3-lite`) |
| `VOYAGE_API_KEY` | - | Voyage AI API key |
| `OPENAI_API_KEY` | - | OpenAI API key |

## Performance

OpenIntel is designed for **local agent workloads** (thousands to low millions of entries):

- **SQLite** — 1M+ entries on commodity hardware
- **Rust** — zero-copy deserialization with `serde`
- **Single file** — no network overhead, no connection pools
- **Small binary** — ~5MB release build
- **Zero runtime dependencies** — SQLite is bundled

For 100B+ scale, use a cloud vector DB. For everything else, use OpenIntel.

## Use Cases

- **Autonomous agents** — memory and knowledge retrieval
- **Trading systems** — market intelligence + trade journaling
- **Research pipelines** — collect, tag, and search findings
- **Newsletter analysis** — archive and semantically search content
- **Competitive intelligence** — track competitor moves with confidence scores
- **Personal knowledge base** — your second brain, embedded

## Contributing

We welcome contributions! This project is maintained by [Kloudy-Sky](https://github.com/Kloudy-Sky).

1. Fork the repo
2. Create a feature branch (`git checkout -b feat/my-feature`)
3. Run tests: `cargo test`
4. Format code: `cargo fmt`
5. Lint: `cargo clippy`
6. Commit and push your changes
7. Open a PR

## Development

```bash
# Run tests
cargo test

# Run with debug logging
RUST_LOG=debug cargo run -- stats

# Format code
cargo fmt

# Lint
cargo clippy

# Build optimized release
cargo build --release
```

## License

MIT — see [Cargo.toml](Cargo.toml) for details.

---

Built with 🎩 by [Jarvis](https://github.com/jrvsai) at [Kloudy-Sky](https://github.com/Kloudy-Sky)
