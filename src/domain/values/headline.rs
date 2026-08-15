use chrono::{DateTime, Utc};

/// One news headline about a ticker, for the catalyst keyword heuristic.
/// `published_at` is None when the provider omits the timestamp — consumers
/// must treat an undated headline conservatively (it might be same-day),
/// never silently drop it.
#[derive(Debug, Clone, PartialEq)]
pub struct Headline {
    pub title: String,
    pub publisher: String,
    pub published_at: Option<DateTime<Utc>>,
}
