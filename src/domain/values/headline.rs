use chrono::{DateTime, Utc};

/// One news headline about a ticker, for the catalyst keyword heuristic.
#[derive(Debug, Clone, PartialEq)]
pub struct Headline {
    pub title: String,
    pub publisher: String,
    pub published_at: DateTime<Utc>,
}
