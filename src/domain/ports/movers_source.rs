use async_trait::async_trait;

use crate::domain::error::DomainError;
use crate::domain::values::mover::MoverRow;

/// The day's biggest percentage losers, worst first. Universe feed for
/// dip_scan; symbol-scoped ports stay untouched.
#[async_trait]
pub trait MoversSource: Send + Sync {
    fn name(&self) -> &str;
    async fn day_losers(&self, count: usize) -> Result<Vec<MoverRow>, DomainError>;
}
