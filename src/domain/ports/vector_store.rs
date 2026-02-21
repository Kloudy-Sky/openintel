pub trait VectorStore: Send + Sync {
    fn store(&self, id: &str, vector: &[f32]) -> Result<(), String>;
    fn search_similar(&self, vector: &[f32], limit: usize) -> Result<Vec<(String, f64)>, String>;
    fn has_vector(&self, id: &str) -> Result<bool, String>;
}
