use async_trait::async_trait;
use chrono::NaiveDate;

use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::values::filing::Filing;

/// Regulatory filings on/after `since` for a ticker. An error (unreachable
/// registry, ticker not a mapped filer) means "could not verify" — callers
/// must fail closed, never treat it as "no filings".
#[async_trait]
pub trait FilingsSource: Send + Sync {
    async fn recent_filings(
        &self,
        ticker: &Ticker,
        since: NaiveDate,
    ) -> Result<Vec<Filing>, DomainError>;
}
