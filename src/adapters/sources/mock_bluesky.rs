use async_trait::async_trait;
use chrono::{TimeZone, Utc};

use crate::domain::entities::social_post::{PostText, SocialPost};
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::ports::social_data_source::SocialDataSource;
use crate::domain::values::source_kind::SourceKind;

pub struct MockBlueskySource;

#[async_trait]
impl SocialDataSource for MockBlueskySource {
    fn kind(&self) -> SourceKind {
        SourceKind::Bluesky
    }

    async fn fetch(&self, ticker: &Ticker, limit: usize) -> Result<Vec<SocialPost>, DomainError> {
        let sym = ticker.as_str();
        let fixtures = [
            (
                "bsky-1",
                "indexfan",
                format!("{sym} looking bullish into the print"),
                22u32,
            ),
            (
                "bsky-2",
                "skeptic",
                format!("not sold on {sym}, might sell my shares"),
                9,
            ),
            (
                "bsky-3",
                "daytripper",
                format!("{sym} green day, up big"),
                14,
            ),
        ];
        fixtures
            .iter()
            .take(limit)
            .map(|(id, author, text, eng)| {
                Ok(SocialPost {
                    id: (*id).to_string(),
                    source: SourceKind::Bluesky,
                    author: (*author).to_string(),
                    text: PostText::parse(text)?,
                    created_at: Utc.with_ymd_and_hms(2026, 6, 24, 16, 0, 0).unwrap(),
                    engagement: *eng,
                })
            })
            .collect::<Result<Vec<_>, DomainError>>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn returns_posts() {
        let posts = MockBlueskySource
            .fetch(&Ticker::parse("AAPL").unwrap(), 50)
            .await
            .unwrap();
        assert_eq!(posts.len(), 3);
        assert_eq!(MockBlueskySource.kind(), SourceKind::Bluesky);
    }
}
