use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::domain::error::DomainError;
use crate::domain::values::source_kind::SourceKind;

pub(crate) const MAX_POST_LEN: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct PostText(String);

impl PostText {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(DomainError::InvalidPostText("empty".into()));
        }
        if trimmed.chars().count() > MAX_POST_LEN {
            return Err(DomainError::InvalidPostText("exceeds max length".into()));
        }
        Ok(PostText(trimmed.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SocialPost {
    pub id: String,
    pub source: SourceKind,
    pub author: String,
    pub text: PostText,
    pub created_at: DateTime<Utc>,
    pub engagement: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn post_text_trims_and_rejects_empty() {
        assert_eq!(PostText::parse("  hello  ").unwrap().as_str(), "hello");
        assert!(PostText::parse("   ").is_err());
        assert!(PostText::parse(&"x".repeat(10_001)).is_err());
    }

    #[test]
    fn post_text_length_limit_counts_chars_not_bytes() {
        // 10_000 two-byte chars = 20_000 bytes but exactly 10_000 chars -> accepted
        let multibyte = "é".repeat(10_000);
        assert!(PostText::parse(&multibyte).is_ok());
        // 10_001 chars -> rejected
        assert!(PostText::parse(&"é".repeat(10_001)).is_err());
    }
}
