pub mod kalshi;
pub mod nws;
pub mod yahoo;

use crate::domain::entities::intel_entry::IntelEntry;
use async_trait::async_trait;

/// A data feed that produces intel entries from an external source.
#[async_trait]
pub trait Feed: Send + Sync {
    /// Human-readable name of this feed.
    fn name(&self) -> &str;

    /// Fetch data and return intel entries ready to be added.
    async fn fetch(&self) -> Result<Vec<IntelEntry>, FeedError>;
}

#[derive(Debug)]
pub enum FeedError {
    /// HTTP or network error
    Network(String),
    /// Response parsing error
    Parse(String),
    /// Configuration error (missing API key, etc.)
    Config(String),
}

impl std::fmt::Display for FeedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FeedError::Network(msg) => write!(f, "Network error: {msg}"),
            FeedError::Parse(msg) => write!(f, "Parse error: {msg}"),
            FeedError::Config(msg) => write!(f, "Config error: {msg}"),
        }
    }
}

impl std::error::Error for FeedError {}

/// Result of running a feed — how many entries were new vs deduped.
#[derive(Debug, serde::Serialize)]
pub struct FeedResult {
    pub feed_name: String,
    pub entries_fetched: usize,
    pub entries_added: usize,
    pub entries_deduped: usize,
    pub errors: Vec<String>,
}
