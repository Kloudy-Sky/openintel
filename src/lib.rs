pub mod application;
pub mod cli;
pub mod domain;
pub mod infrastructure;

use crate::application::add_intel::AddIntelUseCase;
use crate::application::query::QueryUseCase;
use crate::application::reindex::ReindexUseCase;
use crate::application::search::SearchUseCase;
use crate::application::stats::StatsUseCase;
use crate::application::trade::TradeUseCase;
use crate::domain::ports::embedding_port::EmbeddingProvider;
use crate::domain::ports::intel_repository::IntelRepository;
use crate::domain::ports::trade_repository::TradeRepository;
use crate::domain::ports::vector_store::VectorStore;
use crate::infrastructure::embeddings::noop::NoopProvider;
use crate::infrastructure::embeddings::openai::OpenAiProvider;
use crate::infrastructure::embeddings::voyage::VoyageProvider;
use crate::infrastructure::sqlite::intel_repo::SqliteIntelRepo;
use crate::infrastructure::sqlite::migrations::run_migrations;
use crate::infrastructure::sqlite::trade_repo::SqliteTradeRepo;
use crate::infrastructure::sqlite::vector_store::SqliteVectorStore;
use rusqlite::Connection;
use std::sync::Arc;

pub struct OpenIntel {
    pub add_intel: AddIntelUseCase,
    pub search: SearchUseCase,
    pub query: QueryUseCase,
    pub trade: TradeUseCase,
    pub stats: StatsUseCase,
    pub reindex: ReindexUseCase,
}

impl OpenIntel {
    pub fn new(db_path: &str) -> Result<Self, String> {
        let provider = std::env::var("OPENINTEL_EMBEDDING_PROVIDER").unwrap_or_else(|_| "noop".into());
        let api_key = std::env::var("OPENINTEL_EMBEDDING_API_KEY").unwrap_or_default();
        let model = std::env::var("OPENINTEL_EMBEDDING_MODEL").ok();

        let embedder: Arc<dyn EmbeddingProvider> = match provider.as_str() {
            "voyage" => Arc::new(VoyageProvider::new(api_key, model)),
            "openai" => Arc::new(OpenAiProvider::new(api_key, model)),
            _ => Arc::new(NoopProvider),
        };

        Self::with_providers(db_path, embedder)
    }

    pub fn with_providers(
        db_path: &str,
        embedder: Arc<dyn EmbeddingProvider>,
    ) -> Result<Self, String> {
        // Open 3 connections for different repos (SQLite handles concurrent reads)
        let conn1 = Connection::open(db_path).map_err(|e| format!("DB error: {e}"))?;
        let conn2 = Connection::open(db_path).map_err(|e| format!("DB error: {e}"))?;
        let conn3 = Connection::open(db_path).map_err(|e| format!("DB error: {e}"))?;

        // Run migrations on first connection
        run_migrations(&conn1)?;

        let intel_repo: Arc<dyn IntelRepository> = Arc::new(SqliteIntelRepo::new(conn1));
        let trade_repo: Arc<dyn TradeRepository> = Arc::new(SqliteTradeRepo::new(conn2));
        let vector_store: Arc<dyn VectorStore> = Arc::new(SqliteVectorStore::new(conn3));

        Ok(Self {
            add_intel: AddIntelUseCase::new(intel_repo.clone(), embedder.clone(), vector_store.clone()),
            search: SearchUseCase::new(intel_repo.clone(), embedder.clone(), vector_store.clone()),
            query: QueryUseCase::new(intel_repo.clone()),
            trade: TradeUseCase::new(trade_repo),
            stats: StatsUseCase::new(intel_repo.clone()),
            reindex: ReindexUseCase::new(intel_repo, embedder, vector_store),
        })
    }
}
