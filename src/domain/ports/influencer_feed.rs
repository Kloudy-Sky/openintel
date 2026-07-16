use async_trait::async_trait;

use crate::domain::entities::pulse::PulsePost;
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;

/// Author-filtered event feed (the X Pulse port). Paid upstream — callers
/// invoke it only on explicit user opt-in.
#[async_trait]
pub trait InfluencerFeed: Send + Sync {
    async fn pulse(
        &self,
        ticker: &Ticker,
        accounts: &[String],
        hours_back: u32,
        limit: usize,
    ) -> Result<Vec<PulsePost>, DomainError>;
}
