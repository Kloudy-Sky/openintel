use crate::domain::values::category::Category;
use crate::domain::values::confidence::Confidence;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntelEntry {
    pub id: String,
    pub category: Category,
    pub title: String,
    pub body: String,
    pub source: Option<String>,
    pub tags: Vec<String>,
    pub confidence: Confidence,
    pub actionable: bool,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl IntelEntry {
    pub fn new(
        category: Category,
        title: String,
        body: String,
        source: Option<String>,
        tags: Vec<String>,
        confidence: Confidence,
        actionable: bool,
        metadata: Option<serde_json::Value>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            category,
            title,
            body,
            source,
            tags,
            confidence,
            actionable,
            metadata,
            created_at: now,
            updated_at: now,
        }
    }

    /// Text representation for embedding/search
    pub fn searchable_text(&self) -> String {
        format!("{} {}", self.title, self.body)
    }
}
