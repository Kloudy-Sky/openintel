use async_trait::async_trait;

use crate::domain::entities::social_post::SocialPost;
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::values::source_kind::SourceKind;

#[async_trait]
pub trait SocialDataSource: Send + Sync {
    fn kind(&self) -> SourceKind;
    async fn fetch(&self, ticker: &Ticker, limit: usize) -> Result<Vec<SocialPost>, DomainError>;
}
