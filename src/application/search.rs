use crate::domain::entities::intel_entry::IntelEntry;
use crate::domain::ports::embedding_port::{EmbeddingProvider, InputType};
use crate::domain::ports::intel_repository::IntelRepository;
use crate::domain::ports::vector_store::VectorStore;
use std::collections::HashMap;
use std::sync::Arc;

pub struct SearchUseCase {
    repo: Arc<dyn IntelRepository>,
    embedder: Arc<dyn EmbeddingProvider>,
    vector_store: Arc<dyn VectorStore>,
}

impl SearchUseCase {
    pub fn new(
        repo: Arc<dyn IntelRepository>,
        embedder: Arc<dyn EmbeddingProvider>,
        vector_store: Arc<dyn VectorStore>,
    ) -> Self {
        Self { repo, embedder, vector_store }
    }

    /// Keyword search (FTS)
    pub fn keyword_search(&self, text: &str, limit: usize) -> Result<Vec<IntelEntry>, String> {
        self.repo.search(text, limit)
    }

    /// Semantic (vector) search
    pub async fn semantic_search(&self, query: &str, limit: usize) -> Result<Vec<IntelEntry>, String> {
        let vectors = self.embedder.embed(&[query.to_string()], InputType::Query).await?;
        if vectors.is_empty() {
            return Ok(vec![]);
        }
        let results = self.vector_store.search_similar(&vectors[0], limit)?;
        let mut entries = Vec::new();
        for (id, _score) in results {
            if let Some(entry) = self.repo.get_by_id(&id)? {
                entries.push(entry);
            }
        }
        Ok(entries)
    }

    /// Hybrid search using Reciprocal Rank Fusion (RRF)
    /// 70% semantic + 30% keyword weighting
    pub async fn hybrid_search(&self, query: &str, limit: usize) -> Result<Vec<IntelEntry>, String> {
        let k = 60.0_f64; // RRF constant

        // Get more candidates than needed for better fusion
        let fetch_limit = limit * 3;

        // Keyword results
        let keyword_results = self.repo.search(query, fetch_limit)?;

        // Semantic results
        let semantic_ids: Vec<(String, f64)> = match self.embedder.embed(&[query.to_string()], InputType::Query).await {
            Ok(vectors) if !vectors.is_empty() => {
                self.vector_store.search_similar(&vectors[0], fetch_limit)?
            }
            _ => vec![],
        };

        // RRF scoring
        let mut scores: HashMap<String, f64> = HashMap::new();
        let mut entries_map: HashMap<String, IntelEntry> = HashMap::new();

        // Semantic scores (weight 0.7)
        for (rank, (id, _)) in semantic_ids.iter().enumerate() {
            let rrf = 0.7 / (k + rank as f64 + 1.0);
            *scores.entry(id.clone()).or_default() += rrf;
            if !entries_map.contains_key(id) {
                if let Some(entry) = self.repo.get_by_id(id)? {
                    entries_map.insert(id.clone(), entry);
                }
            }
        }

        // Keyword scores (weight 0.3)
        for (rank, entry) in keyword_results.iter().enumerate() {
            let rrf = 0.3 / (k + rank as f64 + 1.0);
            *scores.entry(entry.id.clone()).or_default() += rrf;
            entries_map.entry(entry.id.clone()).or_insert_with(|| entry.clone());
        }

        // Sort by combined score
        let mut sorted: Vec<_> = scores.into_iter().collect();
        sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let results: Vec<IntelEntry> = sorted
            .into_iter()
            .take(limit)
            .filter_map(|(id, _)| entries_map.remove(&id))
            .collect();

        Ok(results)
    }
}
