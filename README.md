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
| `--enable-reddit/--enable-x/--enable-bluesky` | Restrict to these sources (none given → all enabled) |
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
