use async_trait::async_trait;

use crate::domain::error::DomainError;
use crate::domain::values::mover::{MoverRow, ScreenKind};

/// Predefined mover screens (gainers / losers / most active), magnitude-first
/// as the provider ranks them. Universe feed for dip_scan and discover;
/// symbol-scoped ports stay untouched.
#[async_trait]
pub trait MoversSource: Send + Sync {
    fn name(&self) -> &str;
    async fn screen(&self, kind: ScreenKind, count: usize) -> Result<Vec<MoverRow>, DomainError>;
}
