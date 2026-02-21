use crate::domain::entities::intel_entry::IntelEntry;
use crate::domain::error::DomainError;
use crate::domain::ports::embedding_port::{EmbeddingProvider, InputType};
use crate::domain::ports::intel_repository::IntelRepository;
use crate::domain::ports::vector_store::VectorStore;
use crate::domain::values::category::Category;
use crate::domain::values::confidence::Confidence;
use std::sync::Arc;

pub struct AddIntelUseCase {
    repo: Arc<dyn IntelRepository>,
    embedder: Arc<dyn EmbeddingProvider>,
    vector_store: Arc<dyn VectorStore>,
}

impl AddIntelUseCase {
    pub fn new(
        repo: Arc<dyn IntelRepository>,
        embedder: Arc<dyn EmbeddingProvider>,
        vector_store: Arc<dyn VectorStore>,
    ) -> Self {
        Self {
            repo,
            embedder,
            vector_store,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn execute(
        &self,
        category: Category,
        title: String,
        body: String,
        source: Option<String>,
        tags: Vec<String>,
        confidence: Option<f64>,
        actionable: Option<bool>,
        metadata: Option<serde_json::Value>,
    ) -> Result<IntelEntry, DomainError> {
        let conf = Confidence::new(confidence.unwrap_or(0.5)).map_err(DomainError::InvalidInput)?;
        let entry = IntelEntry::new(
            category,
            title,
            body,
            source,
            tags,
            conf,
            actionable.unwrap_or(false),
            metadata,
        );

        self.repo.add(&entry)?;

        // Try to embed — don't fail the add if embedding fails
        let text = entry.searchable_text();
        match self.embedder.embed(&[text], InputType::Document).await {
            Ok(vectors) if !vectors.is_empty() => {
                let _ = self.vector_store.store(&entry.id, &vectors[0]);
            }
            _ => {}
        }

        Ok(entry)
    }
}
