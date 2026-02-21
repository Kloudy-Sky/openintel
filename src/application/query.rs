use crate::domain::entities::intel_entry::IntelEntry;
use crate::domain::ports::intel_repository::{IntelRepository, QueryFilter};
use crate::domain::values::category::Category;
use chrono::{DateTime, Utc};
use std::sync::Arc;

pub struct QueryUseCase {
    repo: Arc<dyn IntelRepository>,
}

impl QueryUseCase {
    pub fn new(repo: Arc<dyn IntelRepository>) -> Self {
        Self { repo }
    }

    pub fn execute(
        &self,
        category: Option<Category>,
        tag: Option<String>,
        since: Option<DateTime<Utc>>,
        limit: Option<usize>,
    ) -> Result<Vec<IntelEntry>, String> {
        self.repo.query(&QueryFilter { category, tag, since, limit })
    }
}
