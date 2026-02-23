use crate::domain::entities::trade::Trade;
use crate::domain::error::DomainError;
use crate::domain::values::trade_outcome::TradeOutcome;
use async_trait::async_trait;

/// Result of checking whether a trade has been resolved.
#[derive(Debug, Clone)]
pub struct ResolutionResult {
    pub trade_id: String,
    pub resolved: bool,
    pub outcome: Option<TradeOutcome>,
    pub pnl_cents: Option<i64>,
    pub exit_price: Option<f64>,
    pub reason: String,
}

/// Pluggable source for checking trade resolution.
/// Implementations can poll Kalshi, Yahoo Finance, IBKR, etc.
#[async_trait]
pub trait ResolutionSource: Send + Sync {
    /// Name of this resolution source (e.g., "kalshi", "ibkr", "yahoo")
    fn name(&self) -> &str;

    /// Check if a trade has been resolved.
    async fn check(&self, trade: &Trade) -> Result<Option<ResolutionResult>, DomainError>;
}
