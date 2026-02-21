use crate::domain::ports::embedding_port::{EmbeddingProvider, InputType};

pub struct NoopProvider;

#[async_trait::async_trait]
impl EmbeddingProvider for NoopProvider {
    async fn embed(&self, texts: &[String], _input_type: InputType) -> Result<Vec<Vec<f32>>, String> {
        // Return empty vectors — signals no embedding available
        Ok(texts.iter().map(|_| vec![]).collect())
    }

    fn dimension(&self) -> usize {
        0
    }
}
