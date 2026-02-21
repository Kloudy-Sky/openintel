use crate::domain::entities::intel_entry::IntelEntry;
use crate::domain::values::category::Category;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Default)]
pub struct QueryFilter {
    pub category: Option<Category>,
    pub tag: Option<String>,
    pub since: Option<DateTime<Utc>>,
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
    fn add(&self, entry: &IntelEntry) -> Result<(), String>;
    fn query(&self, filter: &QueryFilter) -> Result<Vec<IntelEntry>, String>;
    fn search(&self, text: &str, limit: usize) -> Result<Vec<IntelEntry>, String>;
    fn get_by_id(&self, id: &str) -> Result<Option<IntelEntry>, String>;
    fn stats(&self) -> Result<IntelStats, String>;
    fn tags(&self, category: Option<Category>) -> Result<Vec<TagCount>, String>;
    fn entries_missing_vectors(&self) -> Result<Vec<IntelEntry>, String>;
}
