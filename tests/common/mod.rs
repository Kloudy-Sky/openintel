//! Shared test helpers.

use openintel::domain::entities::intel_entry::IntelEntry;
use openintel::domain::values::category::Category;
use openintel::domain::values::confidence::Confidence;
use openintel::domain::values::source_type::SourceType;
use openintel::infrastructure::embeddings::noop::NoopProvider;
use openintel::OpenIntel;
use std::sync::Arc;

pub fn setup() -> OpenIntel {
    OpenIntel::with_providers(":memory:", Arc::new(NoopProvider)).unwrap()
}

pub fn make_entry(
    title: &str,
    body: &str,
    tags: Vec<&str>,
    source_type: SourceType,
    source: Option<&str>,
    actionable: bool,
) -> IntelEntry {
    IntelEntry::new(
        Category::Market,
        title.to_string(),
        body.to_string(),
        source.map(|s| s.to_string()),
        tags.into_iter().map(|t| t.to_string()).collect(),
        Confidence::default(),
        actionable,
        source_type,
        None,
    )
}
