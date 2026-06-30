use async_trait::async_trait;

use crate::domain::entities::social_post::SocialPost;
use crate::domain::error::DomainError;
use crate::domain::ports::post_analyzer::PostAnalyzer;
use crate::domain::values::polarity::Polarity;
use crate::domain::values::post_signal::PostSignal;

const BULL: &[&str] = &[
    "moon", "calls", "long", "buy", "bullish", "squeeze", "breakout", "rocket", "pump", "rip",
    "green", "up", "rally", "bull",
];
const BEAR: &[&str] = &[
    "puts",
    "short",
    "sell",
    "bearish",
    "dump",
    "crash",
    "drilling",
    "bagholder",
    "rug",
    "red",
    "down",
    "tank",
    "bear",
];
const JARGON: &[&str] = &[
    "calls",
    "puts",
    "0dte",
    "yolo",
    "leaps",
    "theta",
    "gamma",
    "squeeze",
    "otm",
    "itm",
    "strike",
    "iv",
    "delta",
    "vega",
    "contracts",
];

pub struct LexiconAnalyzer;

impl LexiconAnalyzer {
    pub fn new() -> Self {
        LexiconAnalyzer
    }

    fn score(text: &str) -> PostSignal {
        let lower = text.to_lowercase();
        let tokens: Vec<&str> = lower
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|t| !t.is_empty())
            .collect();

        let bull_hits = tokens.iter().filter(|t| BULL.contains(t)).count() as f64;
        let bear_hits = tokens.iter().filter(|t| BEAR.contains(t)).count() as f64;
        let polarity = if bull_hits + bear_hits == 0.0 {
            0.0
        } else {
            (bull_hits - bear_hits) / (bull_hits + bear_hits)
        };
        let speculative = tokens.iter().any(|t| JARGON.contains(t));

        PostSignal {
            polarity: Polarity::new(polarity),
            speculative,
        }
    }
}

impl Default for LexiconAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PostAnalyzer for LexiconAnalyzer {
    async fn analyze(&self, posts: &[SocialPost]) -> Result<Vec<PostSignal>, DomainError> {
        Ok(posts.iter().map(|p| Self::score(p.text.as_str())).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::values::source_kind::SourceKind;
    use chrono::Utc;

    fn post(text: &str) -> SocialPost {
        SocialPost {
            id: "1".into(),
            source: SourceKind::Reddit,
            author: "a".into(),
            text: crate::domain::entities::social_post::PostText::parse(text).unwrap(),
            created_at: Utc::now(),
            engagement: 0,
        }
    }

    #[tokio::test]
    async fn scores_sentiment_and_speculation() {
        let analyzer = LexiconAnalyzer::new();
        let posts = vec![
            post("to the moon, buying calls"),   // bullish + speculative
            post("this will dump, buying puts"), // bearish + speculative
            post("the company released a quarterly report"), // neutral, no jargon
        ];
        let signals = analyzer.analyze(&posts).await.unwrap();
        assert_eq!(signals.len(), 3);
        assert!(signals[0].polarity.value() > 0.0 && signals[0].speculative);
        assert!(signals[1].polarity.value() < 0.0 && signals[1].speculative);
        assert_eq!(signals[2].polarity.value(), 0.0);
        assert!(!signals[2].speculative);
    }
}
