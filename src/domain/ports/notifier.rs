use async_trait::async_trait;

use crate::domain::error::DomainError;

/// One-way push of a short message to the user. Never receives anything back:
/// a channel that could carry an order is an execution rail, and OpenIntel has none.
#[async_trait]
pub trait Notifier: Send + Sync {
    async fn notify(&self, title: &str, body: &str) -> Result<(), DomainError>;
}
