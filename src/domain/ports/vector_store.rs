use crate::domain::error::DomainError;

pub trait VectorStore: Send + Sync {
    fn store(&self, id: &str, vector: &[f32]) -> Result<(), DomainError>;
    fn search_similar(&self, vector: &[f32], limit: usize) -> Result<Vec<(String, f64)>, DomainError>;
    fn has_vector(&self, id: &str) -> Result<bool, DomainError>;
    fn get_stored_dimension(&self) -> Result<Option<usize>, DomainError>;
}
