use crate::domain::values::category::Category;
use crate::domain::values::confidence::Confidence;
use crate::domain::values::source_type::SourceType;
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
    pub source_type: SourceType,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl IntelEntry {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        category: Category,
        title: String,
        body: String,
        source: Option<String>,
        tags: Vec<String>,
        confidence: Confidence,
        actionable: bool,
        source_type: SourceType,
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
            source_type,
            metadata,
            created_at: now,
            updated_at: now,
        }
    }

    /// Confidence adjusted for time decay based on category-specific half-lives.
    pub fn decayed_confidence(&self) -> f64 {
        crate::domain::values::decay::decayed_confidence(
            self.confidence.value(),
            &self.category,
            &self.created_at,
        )
    }

    /// Text representation for embedding/search
    pub fn searchable_text(&self) -> String {
        if self.tags.is_empty() {
            format!("{} {}", self.title, self.body)
        } else {
            format!("{} {} {}", self.title, self.body, self.tags.join(" "))
        }
    }
}
