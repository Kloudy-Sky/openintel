use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "openintel", about = "Structured intelligence knowledge base")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Add an intel entry
    Add {
        /// Category (market, newsletter, social, trading, opportunity, competitor, general)
        category: String,
        /// JSON data with title, body, source, tags, confidence, actionable, source_type, skip_dedup, metadata
        json: String,
    },
    /// Query entries by category
    Query {
        /// Category to filter by
        category: String,
        #[arg(long, default_value = "20")]
        limit: usize,
        /// Legacy date filter (use --from instead)
        #[arg(long, conflicts_with_all = ["from", "last"])]
        since: Option<String>,
        #[arg(long)]
        tag: Option<String>,
        /// Start of date range (ISO-8601)
        #[arg(long, conflicts_with = "last")]
        from: Option<String>,
        /// End of date range (ISO-8601)
        #[arg(long)]
        to: Option<String>,
        /// Relative time window (e.g. 24h, 7d, 30m)
        #[arg(long)]
        last: Option<String>,
        /// Exclude internal (agent-generated) entries
        #[arg(long)]
        exclude_internal: bool,
        /// Sort results by time-decayed confidence (most relevant first)
        #[arg(long)]
        decay: bool,
    },
    /// Keyword search
    Search {
        text: String,
        #[arg(long, default_value = "10")]
        limit: usize,
        /// Start of date range (ISO-8601)
        #[arg(long, conflicts_with = "last")]
        from: Option<String>,
        /// End of date range (ISO-8601)
        #[arg(long)]
        to: Option<String>,
        /// Relative time window (e.g. 24h, 7d, 30m)
        #[arg(long)]
        last: Option<String>,
    },
    /// Semantic (vector) search
    Semantic {
        query: String,
        #[arg(long, default_value = "10")]
        limit: usize,
    },
    /// Hybrid search (semantic + keyword with RRF)
    Think {
        query: String,
        #[arg(long, default_value = "10")]
        limit: usize,
    },
    /// Show database statistics
    Stats,
    /// List tags with counts
    Tags {
        /// Optional category filter
        category: Option<String>,
    },
    /// Add a trade
    TradeAdd {
        /// JSON with ticker, series_ticker, direction, contracts, entry_price, thesis
        json: String,
    },
    /// Resolve a trade
    TradeResolve {
        /// Trade ID
        id: String,
        /// Outcome (win, loss, scratch)
        outcome: String,
        /// P&L in cents
        pnl_cents: i64,
        /// Optional exit price
        #[arg(long)]
        exit_price: Option<f64>,
    },
    /// List trades
    Trades {
        #[arg(long, default_value = "20")]
        limit: usize,
        /// Legacy date filter (use --from instead)
        #[arg(long, conflicts_with_all = ["from", "last"])]
        since: Option<String>,
        #[arg(long)]
        resolved: Option<bool>,
        /// Start of date range (ISO-8601)
        #[arg(long, conflicts_with = "last")]
        from: Option<String>,
        /// End of date range (ISO-8601)
        #[arg(long)]
        to: Option<String>,
        /// Relative time window (e.g. 24h, 7d, 30m)
        #[arg(long)]
        last: Option<String>,
    },
    /// Export entries as JSON
    Export {
        /// Legacy date filter (use --from instead)
        #[arg(long, conflicts_with_all = ["from", "last"])]
        since: Option<String>,
        #[arg(long)]
        category: Option<String>,
        /// Start of date range (ISO-8601)
        #[arg(long, conflicts_with = "last")]
        from: Option<String>,
        /// End of date range (ISO-8601)
        #[arg(long)]
        to: Option<String>,
        /// Relative time window (e.g. 24h, 7d, 30m)
        #[arg(long)]
        last: Option<String>,
        /// Exclude internal (agent-generated) entries
        #[arg(long)]
        exclude_internal: bool,
    },
    /// Generate a daily intelligence summary/briefing
    Summarize {
        /// Hours to look back (default: 24)
        #[arg(long, default_value = "24")]
        hours: u32,
    },
    /// Scan for signal patterns and generate alerts
    Scan {
        /// Hours to look back (default: 24)
        #[arg(long, default_value = "24")]
        hours: u32,
    },
    /// Show pending (unresolved) trades
    Pending,
    /// Reindex entries missing vector embeddings
    Reindex,
    /// Scan for trading opportunities using all registered strategies
    Opportunities {
        /// Hours to look back (default: 48)
        #[arg(long, default_value = "48")]
        hours: u32,
        /// Minimum score threshold
        #[arg(long)]
        min_score: Option<f64>,
        /// Maximum intel entries to scan (default: 500)
        #[arg(long)]
        entry_limit: Option<usize>,
        /// Maximum opportunities to return
        #[arg(long)]
        limit: Option<usize>,
        /// Bankroll in cents for Kelly criterion position sizing
        #[arg(long)]
        bankroll: Option<u64>,
        /// Kelly fraction (0.0–1.0). Default: 0.5 (half-Kelly)
        #[arg(long)]
        kelly_fraction: Option<f64>,
        /// Maximum position size in cents (overrides Kelly). Default: 2500 ($25)
        #[arg(long)]
        max_position: Option<u64>,
    },
    /// Show unified portfolio view across exchanges
    Portfolio {
        /// JSON array of positions: [{"exchange":"kalshi","ticker":"KXBTC-123","direction":"yes","quantity":10,"cost_basis":100}]
        positions_json: String,
        /// Concentration warning threshold (0.0–1.0). Default: 0.5 (50%)
        #[arg(long, default_value = "0.5")]
        threshold: f64,
    },
    /// Calculate Kelly criterion position size for a single trade
    Kelly {
        /// Estimated probability of winning (0.0–1.0)
        #[arg(long)]
        probability: f64,
        /// Current market price in cents (1–99)
        #[arg(long)]
        market_price: f64,
        /// Available bankroll in cents
        #[arg(long)]
        bankroll: u64,
        /// Kelly fraction (0.0–1.0). Default: 0.5 (half-Kelly)
        #[arg(long)]
        kelly_fraction: Option<f64>,
        /// Maximum position size in cents. Default: 2500 ($25)
        #[arg(long)]
        max_position: Option<u64>,
    },
}
