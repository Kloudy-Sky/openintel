use async_trait::async_trait;
use chrono::NaiveDate;

use crate::domain::error::DomainError;
use crate::domain::ports::earnings_calendar_source::EarningsCalendarSource;
use crate::domain::values::earnings_row::EarningsRow;

/// Test double: a fixed calendar day, or a fixed failure (for fail-closed paths).
pub struct MockEarningsCalendar(pub Result<Vec<EarningsRow>, String>);

#[async_trait]
impl EarningsCalendarSource for MockEarningsCalendar {
    async fn on(&self, _date: NaiveDate) -> Result<Vec<EarningsRow>, DomainError> {
        match &self.0 {
            Ok(rows) => Ok(rows.clone()),
            Err(message) => Err(DomainError::SourceFailure {
                name: "mock-calendar".into(),
                message: message.clone(),
            }),
        }
    }
}
