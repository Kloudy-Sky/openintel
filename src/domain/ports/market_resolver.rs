/// Market resolver port — resolves abstract tickers to tradeable contracts with prices.
///
/// Strategies produce opportunities with series-level tickers (e.g., "KXHIGHNY", "IONQ")
/// but no specific contract or price. The resolver maps these to actionable trades:
/// - Kalshi series → specific contract ticker + current market price (1–99¢)
/// - Stock tickers → current price from feed data (in dollar-cents)
///
/// Note on price semantics: Kalshi prices are binary contract probabilities (1–99¢),
/// while equity prices are absolute values in cents ($38.00 → 3800). Kelly criterion
/// sizing only applies to Kalshi-style binary contracts. The execute pipeline checks
/// the exchange field before applying Kelly.
use serde::Serialize;

/// A resolved market with a specific tradeable contract and price.
#[derive(Debug, Clone, Serialize)]
pub struct ResolvedMarket {
    /// The specific contract ticker (e.g., "KXHIGHNY-26FEB28-B44.5" or "IONQ")
    pub contract_ticker: String,
    /// Current market price in cents.
    /// - Kalshi: 1–99 (binary contract probability)
    /// - Equity: price in dollar-cents (e.g., $38.00 → 3800)
    pub price_cents: f64,
    /// Which exchange/market type this resolves to
    pub exchange: Exchange,
    /// Human-readable description
    pub description: String,
}

/// Supported exchange/market types
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Exchange {
    /// Kalshi prediction market (binary contracts, 1–99¢)
    Kalshi,
    /// Equity markets (NYSE, NASDAQ, etc.)
    Equity,
}

impl std::fmt::Display for Exchange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Exchange::Kalshi => write!(f, "kalshi"),
            Exchange::Equity => write!(f, "equity"),
        }
    }
}

/// Trait for resolving market tickers to tradeable contracts.
///
/// Note: This trait uses `async_fn_in_trait` which prevents `dyn MarketResolver`.
/// Current usage is always concrete (`IntelResolver`). If dynamic dispatch is
/// needed in the future, switch to the `async_trait` crate.
#[allow(async_fn_in_trait)]
pub trait MarketResolver: Send + Sync {
    /// Resolve a ticker to a specific tradeable contract with current price.
    /// Returns None if the ticker can't be resolved.
    async fn resolve(&self, ticker: &str) -> Option<ResolvedMarket>;

    /// Resolver name for logging
    fn name(&self) -> &str;
}
