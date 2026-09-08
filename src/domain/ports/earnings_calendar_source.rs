use async_trait::async_trait;
use chrono::NaiveDate;

use crate::domain::error::DomainError;
use crate::domain::values::earnings_row::EarningsRow;

/// Companies scheduled to report on a date. An empty list means the provider
/// lists nothing that day; an error means "could not verify" and callers must
/// report it as such, never as a quiet calendar.
#[async_trait]
pub trait EarningsCalendarSource: Send + Sync {
    async fn on(&self, date: NaiveDate) -> Result<Vec<EarningsRow>, DomainError>;
}
