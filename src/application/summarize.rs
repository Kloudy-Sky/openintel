use crate::domain::entities::intel_entry::IntelEntry;
use crate::domain::error::DomainError;
use crate::domain::ports::intel_repository::{IntelRepository, QueryFilter};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;

pub struct SummarizeUseCase {
    repo: Arc<dyn IntelRepository>,
}

#[derive(Debug, Serialize)]
pub struct DailySummary {
    pub generated_at: DateTime<Utc>,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub total_entries: usize,
    pub by_category: Vec<CategorySummary>,
    pub top_tags: Vec<TagMention>,
    pub actionable_items: Vec<ActionableItem>,
}

#[derive(Debug, Serialize)]
pub struct CategorySummary {
    pub category: String,
    pub count: usize,
    pub titles: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct TagMention {
    pub tag: String,
    pub count: usize,
}

#[derive(Debug, Serialize)]
pub struct ActionableItem {
    pub id: String,
    pub category: String,
    pub title: String,
    pub confidence: f64,
}

impl SummarizeUseCase {
    pub fn new(repo: Arc<dyn IntelRepository>) -> Self {
        Self { repo }
    }

    pub fn execute(&self, hours: u32) -> Result<DailySummary, DomainError> {
        let now = Utc::now();
        let since = now - chrono::Duration::hours(hours as i64);

        let entries = self.repo.query(&QueryFilter {
            category: None,
            tag: None,
            since: Some(since),
            limit: None,
        })?;

        let total_entries = entries.len();

        // Group by category
        let mut cat_map: HashMap<String, Vec<&IntelEntry>> = HashMap::new();
        for entry in &entries {
            cat_map
                .entry(entry.category.to_string())
                .or_default()
                .push(entry);
        }

        let mut by_category: Vec<CategorySummary> = cat_map
            .into_iter()
            .map(|(cat, entries)| {
                let count = entries.len();
                let titles: Vec<String> = entries.iter().take(10).map(|e| e.title.clone()).collect();
                CategorySummary {
                    category: cat,
                    count,
                    titles,
                }
            })
            .collect();
        by_category.sort_by(|a, b| b.count.cmp(&a.count));

        // Count tag mentions
        let mut tag_counts: HashMap<String, usize> = HashMap::new();
        for entry in &entries {
            for tag in &entry.tags {
                *tag_counts.entry(tag.clone()).or_default() += 1;
            }
        }
        let mut top_tags: Vec<TagMention> = tag_counts
            .into_iter()
            .map(|(tag, count)| TagMention { tag, count })
            .collect();
        top_tags.sort_by(|a, b| b.count.cmp(&a.count));
        top_tags.truncate(20); // Top 20 tags

        // Actionable items sorted by confidence desc
        let mut actionable_items: Vec<ActionableItem> = entries
            .iter()
            .filter(|e| e.actionable)
            .map(|e| ActionableItem {
                id: e.id.clone(),
                category: e.category.to_string(),
                title: e.title.clone(),
                confidence: e.confidence.value(),
            })
            .collect();
        actionable_items.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal));

        Ok(DailySummary {
            generated_at: now,
            period_start: since,
            period_end: now,
            total_entries,
            by_category,
            top_tags,
            actionable_items,
        })
    }
}
