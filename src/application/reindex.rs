use crate::domain::ports::embedding_port::{EmbeddingProvider, InputType};
use crate::domain::ports::intel_repository::IntelRepository;
use crate::domain::ports::vector_store::VectorStore;
use std::sync::Arc;

pub struct ReindexUseCase {
    repo: Arc<dyn IntelRepository>,
    embedder: Arc<dyn EmbeddingProvider>,
    vector_store: Arc<dyn VectorStore>,
}

impl ReindexUseCase {
    pub fn new(
        repo: Arc<dyn IntelRepository>,
        embedder: Arc<dyn EmbeddingProvider>,
        vector_store: Arc<dyn VectorStore>,
    ) -> Self {
        Self { repo, embedder, vector_store }
    }

    pub async fn execute(&self) -> Result<usize, String> {
        let entries = self.repo.entries_missing_vectors()?;
        let total = entries.len();
        if total == 0 {
            return Ok(0);
        }

        // Batch embed in chunks of 32
        for chunk in entries.chunks(32) {
            let texts: Vec<String> = chunk.iter().map(|e| e.searchable_text()).collect();
            let vectors = self.embedder.embed(&texts, InputType::Document).await?;
            for (entry, vector) in chunk.iter().zip(vectors.iter()) {
                self.vector_store.store(&entry.id, vector)?;
            }
        }

        Ok(total)
    }
}
