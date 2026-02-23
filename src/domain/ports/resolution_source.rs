use crate::domain::entities::trade::Trade;
use crate::domain::error::DomainError;
use crate::domain::values::trade_outcome::TradeOutcome;
use async_trait::async_trait;

/// Result of a resolved trade. Returned inside `Some(...)` when a source
/// confirms resolution; `None` means the source cannot determine yet.
#[derive(Debug, Clone)]
pub struct ResolutionResult {
    pub outcome: TradeOutcome,
    pub pnl_cents: i64,
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
    /// Returns `Ok(Some(result))` if resolved, `Ok(None)` if not yet determinable,
    /// or `Err(...)` on transient failure.
    async fn check(&self, trade: &Trade) -> Result<Option<ResolutionResult>, DomainError>;
}
