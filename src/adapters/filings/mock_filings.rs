use async_trait::async_trait;
use chrono::NaiveDate;

use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::ports::filings_source::FilingsSource;
use crate::domain::values::filing::Filing;

/// Test double: a fixed filing list, or a fixed failure (for fail-closed paths).
pub struct MockFilingsSource(pub Result<Vec<Filing>, String>);

#[async_trait]
impl FilingsSource for MockFilingsSource {
    async fn recent_filings(
        &self,
        _ticker: &Ticker,
        since: NaiveDate,
    ) -> Result<Vec<Filing>, DomainError> {
        match &self.0 {
            Ok(filings) => Ok(filings
                .iter()
                .filter(|f| f.filed_on >= since)
                .cloned()
                .collect()),
            Err(message) => Err(DomainError::SourceFailure {
                name: "mock-filings".into(),
                message: message.clone(),
            }),
        }
    }
}
