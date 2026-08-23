use async_trait::async_trait;

use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::values::bar::Bar;

/// Daily OHLC bars for risk math (ATR). Kept separate from
/// `MarketDataSource` so snapshot consumers and mocks are untouched.
#[async_trait]
pub trait BarSource: Send + Sync {
    async fn bars(&self, ticker: &Ticker) -> Result<Vec<Bar>, DomainError>;

    /// Longer daily history when the source can serve it (target ~1 year,
    /// for period-extreme levels). Defaults to the standard window — callers
    /// must report the span they actually received, never assume a year.
    async fn bars_long(&self, ticker: &Ticker) -> Result<Vec<Bar>, DomainError> {
        self.bars(ticker).await
    }
}
