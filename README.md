# OpenIntel

**Structured intelligence knowledge base with hybrid semantic + keyword search.**

OpenIntel is an embedded, file-based intelligence store built on SQLite with optional vector embeddings. It's designed for autonomous agents, trading systems, research pipelines, and anyone who needs structured signal storage with powerful retrieval.

## Why OpenIntel?

Most vector databases are designed for large-scale cloud deployments. OpenIntel takes the opposite approach — **SQLite-simple, single-file, zero infrastructure.** Think of it as "SQLite for intelligence."

- **Structured entries** with categories, tags, confidence scores, and metadata
- **Hybrid search** — BM25 keyword search + semantic vector similarity (Reciprocal Rank Fusion)
- **Trade tracking** — built-in trade journal with P&L resolution
- **Embedded** — single `.db` file, no server needed
- **Pluggable embeddings** — bring your own embedding provider (Voyage, OpenAI, Cohere, etc.)

## Quick Start

```bash
# Install
npm install openintel

# Or use the CLI directly
npx openintel add market '{"title":"AAPL earnings beat","body":"Revenue up 8% YoY","tags":["AAPL","earnings"]}'
npx openintel search "Apple revenue growth"
npx openintel query market --limit 10
```

## CLI Usage

```bash
# Add an intel entry
openintel add <category> '<json_data>'

# Categories: market, newsletter, social, trading, opportunity, competitor, general

# Search (keyword)
openintel search <query> [--limit N]

# Semantic search (requires embedding provider)
openintel semantic <query> [--limit N]

# Hybrid search (keyword + semantic, fused ranking)
openintel think <query> [--limit N]

# Query by category
openintel query <category> [--limit N] [--since YYYY-MM-DD] [--tag TAG]

# Stats
openintel stats

# Tags
openintel tags [category]

# Trade tracking
openintel trade-add '<json>'
openintel trade-resolve <id> <outcome> <pnl_cents>
openintel trades [--limit N] [--resolved true/false]

# Export
openintel export [--since YYYY-MM-DD] [--category CAT]

# Re-embed entries missing vectors
openintel reindex
```

## Programmatic API

```javascript
const { OpenIntel } = require('openintel');

// Create/open a database
const intel = new OpenIntel({
  dbPath: './my-intel.db',
  embedding: {
    provider: 'voyage',      // or 'openai', 'cohere', 'custom'
    apiKey: process.env.VOYAGE_API_KEY,
    model: 'voyage-3-lite',
    dimensions: 1024
  }
});

// Add an entry
const id = await intel.add('market', {
  title: 'Fed signals rate cut',
  body: 'FOMC minutes suggest 25bp cut likely in March...',
  tags: ['fed', 'rates', 'macro'],
  confidence: 0.8,
  actionable: true
});

// Hybrid search
const results = await intel.think('federal reserve interest rates', { limit: 5 });

// Query by category
const entries = intel.query('market', { limit: 10, since: '2024-01-01', tag: 'fed' });

// Trade tracking
const tradeId = intel.addTrade({
  ticker: 'AAPL',
  direction: 'long',
  contracts: 100,
  entry_price: 185.50,
  thesis: 'Earnings momentum + services growth'
});

intel.resolveTrade(tradeId, 'win', 350); // +$3.50

// Stats
const stats = intel.stats();
console.log(stats); // { total: 157, byCategory: { market: 48, ... }, ... }

// Close
intel.close();
```

## Embedding Providers

OpenIntel supports pluggable embedding providers for semantic search:

| Provider | Model | Dimensions | Notes |
|----------|-------|-----------|-------|
| Voyage AI | voyage-3-lite, voyage-4-lite | 1024 | Best value for money |
| OpenAI | text-embedding-3-small | 1536 | Widely available |
| Cohere | embed-english-v3.0 | 1024 | Good multilingual |
| Custom | Any | Any | Bring your own function |

Without an embedding provider, OpenIntel still works — you just get keyword search instead of semantic/hybrid search.

### Custom embedding function

```javascript
const intel = new OpenIntel({
  dbPath: './my-intel.db',
  embedding: {
    provider: 'custom',
    dimensions: 768,
    embedFn: async (texts, inputType) => {
      // Return array of float arrays
      return texts.map(t => myEmbedder.encode(t));
    }
  }
});
```

## Schema

### Intel Entries

| Field | Type | Description |
|-------|------|-------------|
| id | INTEGER | Auto-incrementing primary key |
| category | TEXT | market, newsletter, social, trading, opportunity, competitor, general |
| title | TEXT | Short descriptive title (required) |
| body | TEXT | Full content/analysis |
| source | TEXT | Where this intel came from |
| tags | JSON | Array of string tags |
| confidence | REAL | 0.0 - 1.0 confidence score |
| actionable | INTEGER | Boolean — is this actionable? |
| metadata | JSON | Arbitrary key-value pairs |
| created_at | TEXT | ISO timestamp |
| expires_at | TEXT | Optional expiration |

### Trade Journal

| Field | Type | Description |
|-------|------|-------------|
| id | INTEGER | Auto-incrementing primary key |
| ticker | TEXT | Asset ticker/symbol |
| series_ticker | TEXT | Series identifier (for event contracts) |
| direction | TEXT | long/short/yes/no |
| contracts | INTEGER | Position size |
| entry_price | REAL | Entry price per unit |
| exit_price | REAL | Exit price (when resolved) |
| thesis | TEXT | Why this trade was made |
| outcome | TEXT | win/loss/scratch |
| pnl_cents | INTEGER | P&L in cents |
| resolved_at | TEXT | When trade was closed |
| created_at | TEXT | When trade was opened |

## Architecture

```
┌─────────────────────────────────────────┐
│              OpenIntel                   │
├─────────────────────────────────────────┤
│  CLI (cli.js)  │  API (index.js)        │
├─────────────────────────────────────────┤
│  Hybrid Search Engine                    │
│  ├── BM25 Keyword (SQLite LIKE)         │
│  ├── Vector Similarity (sqlite-vec)      │
│  └── Reciprocal Rank Fusion (RRF)       │
├─────────────────────────────────────────┤
│  SQLite (better-sqlite3)                │
│  ├── intel table                         │
│  ├── intel_vec (vector index)            │
│  └── kalshi_trades table                 │
├─────────────────────────────────────────┤
│  Embedding Provider (pluggable)          │
│  ├── Voyage AI                           │
│  ├── OpenAI                              │
│  ├── Cohere                              │
│  └── Custom                              │
└─────────────────────────────────────────┘
```

## Contributing

We welcome contributions! This project is maintained by [Kloudy-Sky](https://github.com/Kloudy-Sky).

1. Fork the repo
2. Create a feature branch (`git checkout -b feat/my-feature`)
3. Commit your changes
4. Push and open a PR

## License

Apache 2.0 — see [LICENSE](LICENSE).

---

Built with 🎩 by [Jarvis](https://github.com/jrvsai) at [Kloudy-Sky](https://github.com/Kloudy-Sky)
