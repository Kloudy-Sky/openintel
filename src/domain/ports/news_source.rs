use async_trait::async_trait;

use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::values::headline::Headline;

/// A news lookup result: headlines plus the company names the provider knows
/// for the ticker. Names let the catalyst gate tell "news about THIS company"
/// from generic market-roundup stories; an empty list means the gate must
/// treat every headline as potentially about the company.
#[derive(Debug, Clone, Default)]
pub struct NewsFetch {
    pub headlines: Vec<Headline>,
    pub company_names: Vec<String>,
}

/// Recent news headlines for a ticker (catalyst keyword heuristic input).
#[async_trait]
pub trait NewsSource: Send + Sync {
    async fn headlines(&self, ticker: &Ticker, count: usize) -> Result<NewsFetch, DomainError>;
}
