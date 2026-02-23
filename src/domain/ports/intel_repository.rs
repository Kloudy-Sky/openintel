use crate::domain::entities::intel_entry::IntelEntry;
use crate::domain::error::DomainError;
use crate::domain::values::category::Category;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Default)]
pub struct QueryFilter {
    pub category: Option<Category>,
    pub tag: Option<String>,
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct IntelStats {
    pub total_entries: usize,
    pub by_category: Vec<(String, usize)>,
    pub total_tags: usize,
    pub actionable_count: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TagCount {
    pub tag: String,
    pub count: usize,
}

pub trait IntelRepository: Send + Sync {
    fn add(&self, entry: &IntelEntry) -> Result<(), DomainError>;
    fn query(&self, filter: &QueryFilter) -> Result<Vec<IntelEntry>, DomainError>;
    /// Keyword search with optional time bounds. This is the only required search method.
    fn search_with_time(
        &self,
        text: &str,
        limit: usize,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
    ) -> Result<Vec<IntelEntry>, DomainError>;
    /// Convenience wrapper — delegates to `search_with_time` with no time bounds.
    fn search(&self, text: &str, limit: usize) -> Result<Vec<IntelEntry>, DomainError> {
        self.search_with_time(text, limit, None, None)
    }
    fn get_by_id(&self, id: &str) -> Result<Option<IntelEntry>, DomainError>;
    fn stats(&self) -> Result<IntelStats, DomainError>;
    fn tags(&self, category: Option<Category>) -> Result<Vec<TagCount>, DomainError>;
    fn entries_missing_vectors(&self) -> Result<Vec<IntelEntry>, DomainError>;
}
