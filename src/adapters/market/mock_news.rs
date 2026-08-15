use async_trait::async_trait;

use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::ports::news_source::NewsSource;
use crate::domain::values::headline::Headline;

/// Test double: fixed headlines, or a fixed failure (for fail-closed paths).
pub struct MockNewsSource(pub Result<Vec<Headline>, String>);

#[async_trait]
impl NewsSource for MockNewsSource {
    async fn headlines(
        &self,
        _ticker: &Ticker,
        count: usize,
    ) -> Result<Vec<Headline>, DomainError> {
        match &self.0 {
            Ok(headlines) => Ok(headlines.iter().take(count).cloned().collect()),
            Err(message) => Err(DomainError::SourceFailure {
                name: "mock-news".into(),
                message: message.clone(),
            }),
        }
    }
}
