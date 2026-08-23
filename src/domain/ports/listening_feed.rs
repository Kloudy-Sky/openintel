use async_trait::async_trait;

use crate::domain::entities::pulse::PulsePost;
use crate::domain::error::DomainError;

/// What one ticker-less listening fetch yielded. `posts_returned` is what the
/// upstream API actually returned (= what a pay-per-read API bills), which
/// can exceed `posts.len()` due to client-side skips.
#[derive(Debug, Clone)]
pub struct ListeningFetch {
    pub posts: Vec<PulsePost>,
    pub posts_returned: u32,
}

/// Ticker-less feed for chatter discovery: recent posts from a listening set,
/// no symbol required. `handles` are accounts for X/Bluesky and subreddits
/// for Reddit. Posts reuse `PulsePost` deliberately — like pulse, they never
/// enter the fusion engine's sentiment averaging.
#[async_trait]
pub trait ListeningFeed: Send + Sync {
    /// Stable platform label ("x" / "bluesky" / "reddit") — keys the chatter
    /// baseline journal.
    fn platform(&self) -> &'static str;
    /// True when every read bills real money — callers must gate on explicit
    /// user opt-in and disclose the cost first.
    fn paid(&self) -> bool;
    async fn listening_posts(
        &self,
        handles: &[String],
        hours_back: u32,
        limit: usize,
    ) -> Result<ListeningFetch, DomainError>;
}
