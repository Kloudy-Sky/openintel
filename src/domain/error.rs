use thiserror::Error;

#[derive(Debug, Error)]
pub enum DomainError {
    #[error("invalid ticker: {0}")]
    InvalidTicker(String),

    #[error("invalid post text: {0}")]
    InvalidPostText(String),

    #[error("analyzer returned {got} signals for {expected} posts")]
    AnalyzerMismatch { expected: usize, got: usize },

    #[error("market snapshot ticker '{got}' does not match requested '{expected}'")]
    MarketTickerMismatch { expected: String, got: String },

    #[error("data source '{name}' failed: {message}")]
    SourceFailure { name: String, message: String },

    #[error("no data: no posts and no market snapshot available")]
    NoData,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displays_human_messages() {
        assert_eq!(
            DomainError::InvalidTicker("@@".into()).to_string(),
            "invalid ticker: @@"
        );
        assert_eq!(
            DomainError::AnalyzerMismatch {
                expected: 3,
                got: 2
            }
            .to_string(),
            "analyzer returned 2 signals for 3 posts"
        );
        assert_eq!(
            DomainError::NoData.to_string(),
            "no data: no posts and no market snapshot available"
        );
    }
}
