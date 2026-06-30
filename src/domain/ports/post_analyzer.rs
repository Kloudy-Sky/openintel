use async_trait::async_trait;

use crate::domain::entities::social_post::SocialPost;
use crate::domain::error::DomainError;
use crate::domain::values::post_signal::PostSignal;

#[async_trait]
pub trait PostAnalyzer: Send + Sync {
    /// One `PostSignal` per input post, aligned to input order (`len == posts.len()`).
    async fn analyze(&self, posts: &[SocialPost]) -> Result<Vec<PostSignal>, DomainError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::values::polarity::Polarity;

    struct ConstAnalyzer;

    #[async_trait]
    impl PostAnalyzer for ConstAnalyzer {
        async fn analyze(&self, posts: &[SocialPost]) -> Result<Vec<PostSignal>, DomainError> {
            Ok(posts
                .iter()
                .map(|_| PostSignal {
                    polarity: Polarity::new(0.0),
                    speculative: false,
                })
                .collect())
        }
    }

    #[tokio::test]
    async fn trait_is_object_safe_and_callable() {
        let analyzer: Box<dyn PostAnalyzer> = Box::new(ConstAnalyzer);
        let out = analyzer.analyze(&[]).await.unwrap();
        assert!(out.is_empty());
    }
}
