use async_trait::async_trait;

use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::values::bar::Bar;

/// Daily OHLC bars for risk math (ATR). Kept separate from
/// `MarketDataSource` so snapshot consumers and mocks are untouched.
#[async_trait]
pub trait BarSource: Send + Sync {
    async fn bars(&self, ticker: &Ticker) -> Result<Vec<Bar>, DomainError>;
}
