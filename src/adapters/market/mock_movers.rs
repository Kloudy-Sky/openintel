use async_trait::async_trait;

use crate::domain::error::DomainError;
use crate::domain::ports::movers_source::MoversSource;
use crate::domain::values::mover::{MoverRow, ScreenKind};

/// Test double: one fixed list served for every screen kind.
pub struct MockMoversSource(pub Vec<MoverRow>);

#[async_trait]
impl MoversSource for MockMoversSource {
    fn name(&self) -> &str {
        "mock-movers"
    }

    async fn screen(&self, _kind: ScreenKind, count: usize) -> Result<Vec<MoverRow>, DomainError> {
        Ok(self.0.iter().take(count).cloned().collect())
    }
}
