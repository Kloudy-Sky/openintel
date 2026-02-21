use thiserror::Error;

#[derive(Debug, Error)]
pub enum DomainError {
    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("Database error: {0}")]
    Database(String),

    #[error("Embedding error: {0}")]
    Embedding(String),

    #[error("Parse error: {0}")]
    Parse(String),
}

impl From<String> for DomainError {
    fn from(s: String) -> Self {
        DomainError::Database(s)
    }
}

impl From<&str> for DomainError {
    fn from(s: &str) -> Self {
        DomainError::InvalidInput(s.to_string())
    }
}
