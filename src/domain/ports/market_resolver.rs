/// Market resolver port — resolves abstract tickers to tradeable contracts with prices.
///
/// Strategies produce opportunities with series-level tickers (e.g., "KXHIGHNY", "IONQ")
/// but no specific contract or price. The resolver maps these to actionable trades:
/// - Kalshi series → specific contract ticker + current market price
/// - Stock tickers → current price from feed data in the intel DB
use serde::Serialize;

/// A resolved market with a specific tradeable contract and price.
#[derive(Debug, Clone, Serialize)]
pub struct ResolvedMarket {
    /// The specific contract ticker (e.g., "KXHIGHNY-26FEB28-B44.5" or "IONQ")
    pub contract_ticker: String,
    /// Current market price in cents (1–99 for Kalshi, actual price×100 for stocks)
    pub price_cents: f64,
    /// Which exchange this resolves to
    pub exchange: Exchange,
    /// Human-readable description
    pub description: String,
}

/// Supported exchanges
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Exchange {
    Kalshi,
    Ibkr,
    Yahoo,
}

impl std::fmt::Display for Exchange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Exchange::Kalshi => write!(f, "kalshi"),
            Exchange::Ibkr => write!(f, "ibkr"),
            Exchange::Yahoo => write!(f, "yahoo"),
        }
    }
}

/// Trait for resolving market tickers to tradeable contracts.
#[allow(async_fn_in_trait)]
pub trait MarketResolver: Send + Sync {
    /// Resolve a ticker to a specific tradeable contract with current price.
    /// Returns None if the ticker can't be resolved.
    async fn resolve(&self, ticker: &str) -> Option<ResolvedMarket>;

    /// Resolver name for logging
    fn name(&self) -> &str;
}
