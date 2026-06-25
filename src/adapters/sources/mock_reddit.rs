use async_trait::async_trait;
use chrono::{TimeZone, Utc};

use crate::domain::entities::social_post::{PostText, SocialPost};
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::ports::social_data_source::SocialDataSource;
use crate::domain::values::source_kind::SourceKind;

pub struct MockRedditSource;

#[async_trait]
impl SocialDataSource for MockRedditSource {
    fn kind(&self) -> SourceKind {
        SourceKind::Reddit
    }

    async fn fetch(&self, ticker: &Ticker, limit: usize) -> Result<Vec<SocialPost>, DomainError> {
        let sym = ticker.as_str();
        let fixtures = [
            (
                "reddit-1",
                "dudebro",
                format!("{sym} to the moon, loading calls all day"),
                420u32,
            ),
            (
                "reddit-2",
                "valuepicker",
                format!("{sym} earnings look strong, going long here"),
                88,
            ),
            (
                "reddit-3",
                "chartwatcher",
                format!("{sym} breakout confirmed, rocket time"),
                51,
            ),
            (
                "reddit-4",
                "shortking",
                format!("{sym} is going to dump, buying puts"),
                31,
            ),
        ];
        Ok(fixtures
            .iter()
            .take(limit)
            .map(|(id, author, text, eng)| SocialPost {
                id: (*id).to_string(),
                source: SourceKind::Reddit,
                author: (*author).to_string(),
                text: PostText::parse(text).expect("fixture text is valid"),
                created_at: Utc.with_ymd_and_hms(2026, 6, 24, 14, 0, 0).unwrap(),
                engagement: *eng,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn returns_deterministic_posts_and_honors_limit() {
        let src = MockRedditSource;
        let t = Ticker::parse("AAPL").unwrap();
        let all = src.fetch(&t, 50).await.unwrap();
        assert_eq!(all.len(), 4);
        assert!(all[0].text.as_str().contains("AAPL"));
        assert_eq!(src.fetch(&t, 1).await.unwrap().len(), 1);
        assert_eq!(src.kind(), SourceKind::Reddit);
    }
}
