use async_trait::async_trait;
use chrono::{TimeZone, Utc};

use crate::domain::entities::social_post::{PostText, SocialPost};
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::ports::social_data_source::SocialDataSource;
use crate::domain::values::source_kind::SourceKind;

pub struct MockXSource;

#[async_trait]
impl SocialDataSource for MockXSource {
    fn kind(&self) -> SourceKind {
        SourceKind::X
    }

    async fn fetch(&self, ticker: &Ticker, limit: usize) -> Result<Vec<SocialPost>, DomainError> {
        let sym = ticker.as_str();
        let fixtures = [
            (
                "x-1",
                "quanttrader",
                format!("${sym} squeeze incoming, buying calls"),
                1200u32,
            ),
            (
                "x-2",
                "macroowl",
                format!("watching ${sym} but staying cautious"),
                64,
            ),
            (
                "x-3",
                "trendrider",
                format!("${sym} rally looks strong"),
                240,
            ),
        ];
        Ok(fixtures
            .iter()
            .take(limit)
            .map(|(id, author, text, eng)| SocialPost {
                id: (*id).to_string(),
                source: SourceKind::X,
                author: (*author).to_string(),
                text: PostText::parse(text).expect("fixture text is valid"),
                created_at: Utc.with_ymd_and_hms(2026, 6, 24, 15, 0, 0).unwrap(),
                engagement: *eng,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn returns_posts() {
        let posts = MockXSource
            .fetch(&Ticker::parse("AAPL").unwrap(), 50)
            .await
            .unwrap();
        assert_eq!(posts.len(), 3);
        assert_eq!(MockXSource.kind(), SourceKind::X);
    }
}
