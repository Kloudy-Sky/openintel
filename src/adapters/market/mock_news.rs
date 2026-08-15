use async_trait::async_trait;

use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::ports::news_source::{NewsFetch, NewsSource};

/// Test double: a fixed news fetch, or a fixed failure (for fail-closed paths).
pub struct MockNewsSource(pub Result<NewsFetch, String>);

#[async_trait]
impl NewsSource for MockNewsSource {
    async fn headlines(&self, _ticker: &Ticker, count: usize) -> Result<NewsFetch, DomainError> {
        match &self.0 {
            Ok(fetch) => Ok(NewsFetch {
                headlines: fetch.headlines.iter().take(count).cloned().collect(),
                company_names: fetch.company_names.clone(),
            }),
            Err(message) => Err(DomainError::SourceFailure {
                name: "mock-news".into(),
                message: message.clone(),
            }),
        }
    }
}
