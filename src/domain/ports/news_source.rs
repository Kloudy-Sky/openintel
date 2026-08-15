use async_trait::async_trait;

use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::values::headline::Headline;

/// Recent news headlines for a ticker (catalyst keyword heuristic input).
#[async_trait]
pub trait NewsSource: Send + Sync {
    async fn headlines(&self, ticker: &Ticker, count: usize) -> Result<Vec<Headline>, DomainError>;
}
