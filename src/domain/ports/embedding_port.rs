#[derive(Debug, Clone, Copy)]
pub enum InputType {
    Document,
    Query,
}

#[async_trait::async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, texts: &[String], input_type: InputType) -> Result<Vec<Vec<f32>>, String>;
    fn dimension(&self) -> usize;
}
