use async_trait::async_trait;

use crate::domain::entities::market_snapshot::MarketSnapshot;
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;

#[async_trait]
pub trait MarketDataSource: Send + Sync {
    fn name(&self) -> &'static str;
    async fn snapshot(&self, ticker: &Ticker) -> Result<MarketSnapshot, DomainError>;
}
