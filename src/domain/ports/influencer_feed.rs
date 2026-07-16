use async_trait::async_trait;

use crate::domain::entities::pulse::PulseFetch;
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;

/// Author-filtered event feed (the X Pulse port). Paid upstream — callers
/// invoke it only on explicit user opt-in.
#[async_trait]
pub trait InfluencerFeed: Send + Sync {
    /// `posts_returned` on the result drives cost accounting — it's what the
    /// upstream API billed, not necessarily `posts.len()`.
    async fn pulse(
        &self,
        ticker: &Ticker,
        accounts: &[String],
        hours_back: u32,
        limit: usize,
    ) -> Result<PulseFetch, DomainError>;
}
