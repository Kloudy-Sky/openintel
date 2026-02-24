pub mod application;
pub mod cli;
pub mod domain;
pub mod infrastructure;

use crate::application::add_intel::{AddIntelUseCase, AddResult};
use crate::application::query::QueryUseCase;
use crate::application::reindex::ReindexUseCase;
use crate::application::search::SearchUseCase;
use crate::application::stats::StatsUseCase;
use crate::application::trade::TradeUseCase;
use crate::domain::entities::intel_entry::IntelEntry;
use crate::domain::entities::trade::Trade;
use crate::domain::error::DomainError;
use crate::domain::ports::embedding_port::EmbeddingProvider;
use crate::domain::ports::intel_repository::{IntelRepository, IntelStats, TagCount};
use crate::domain::ports::trade_repository::TradeRepository;
use crate::domain::ports::vector_store::VectorStore;
use crate::domain::values::category::Category;
use crate::domain::values::source_type::SourceType;
use crate::domain::values::trade_direction::TradeDirection;
use crate::domain::values::trade_outcome::TradeOutcome;
use crate::infrastructure::embeddings::noop::NoopProvider;
use crate::infrastructure::embeddings::openai::OpenAiProvider;
use crate::infrastructure::embeddings::voyage::VoyageProvider;
use crate::infrastructure::sqlite::intel_repo::SqliteIntelRepo;
use crate::infrastructure::sqlite::migrations::run_migrations;
use crate::infrastructure::sqlite::trade_repo::SqliteTradeRepo;
use crate::infrastructure::sqlite::vector_store::SqliteVectorStore;
use chrono::{DateTime, Utc};
use rusqlite::Connection;
use std::sync::Arc;

pub struct OpenIntel {
    add_intel_uc: AddIntelUseCase,
    search_uc: SearchUseCase,
    query_uc: QueryUseCase,
    trade_uc: TradeUseCase,
    stats_uc: StatsUseCase,
    reindex_uc: ReindexUseCase,
}

impl OpenIntel {
    pub fn new(db_path: &str) -> Result<Self, DomainError> {
        // Read embedding configuration from environment
        let mut provider = std::env::var("OPENINTEL_EMBEDDING_PROVIDER").ok();
        let model = std::env::var("OPENINTEL_EMBEDDING_MODEL").ok();

        // Read provider-specific API keys
        let voyage_key = std::env::var("VOYAGE_API_KEY").ok();
        let openai_key = std::env::var("OPENAI_API_KEY").ok();

        // Fallback to generic API key for backward compatibility
        let generic_key = std::env::var("OPENINTEL_EMBEDDING_API_KEY").ok();

        // Auto-detect provider if not explicitly set
        if provider.is_none() {
            if voyage_key.is_some() {
                provider = Some("voyage".to_string());
            } else if openai_key.is_some() {
                provider = Some("openai".to_string());
            }
        }

        let provider = provider.unwrap_or_else(|| "noop".to_string());

        let embedder: Arc<dyn EmbeddingProvider> = match provider.as_str() {
            "voyage" => {
                let api_key = voyage_key.or(generic_key).unwrap_or_default();
                if api_key.is_empty() {
                    eprintln!("WARNING: Voyage provider selected but no API key found (set VOYAGE_API_KEY or OPENINTEL_EMBEDDING_API_KEY)");
                }
                let base_url = std::env::var("VOYAGE_API_BASE").ok();
                Arc::new(VoyageProvider::new(api_key, model, base_url))
            }
            "openai" => {
                let api_key = openai_key.or(generic_key).unwrap_or_default();
                if api_key.is_empty() {
                    eprintln!("WARNING: OpenAI provider selected but no API key found (set OPENAI_API_KEY or OPENINTEL_EMBEDDING_API_KEY)");
                }
                Arc::new(OpenAiProvider::new(api_key, model))
            }
            "noop" | "" => Arc::new(NoopProvider),
            unknown => {
                eprintln!(
                    "WARNING: Unknown embedding provider '{}', falling back to noop",
                    unknown
                );
                Arc::new(NoopProvider)
            }
        };

        Self::with_providers(db_path, embedder)
    }

    pub fn with_providers(
        db_path: &str,
        embedder: Arc<dyn EmbeddingProvider>,
    ) -> Result<Self, DomainError> {
        let open = |path: &str| -> Result<Connection, DomainError> {
            let conn = if path == ":memory:" {
                Connection::open_in_memory()
            } else {
                Connection::open(path)
            }
            .map_err(|e| DomainError::Database(format!("DB error: {e}")))?;
            if path != ":memory:" {
                conn.pragma_update(None, "journal_mode", "WAL")
                    .map_err(|e| DomainError::Database(format!("WAL error: {e}")))?;
            }
            run_migrations(&conn)?;
            Ok(conn)
        };

        let conn1 = open(db_path)?;
        let conn2 = open(db_path)?;
        let conn3 = open(db_path)?;

        let intel_repo: Arc<dyn IntelRepository> = Arc::new(SqliteIntelRepo::new(conn1));
        let trade_repo: Arc<dyn TradeRepository> = Arc::new(SqliteTradeRepo::new(conn2));
        let vector_store: Arc<dyn VectorStore> = Arc::new(SqliteVectorStore::new(conn3));

        // Vector dimension validation (Fix #3)
        let provider_dim = embedder.dimension();
        if provider_dim > 0 {
            if let Ok(Some(stored_dim)) = vector_store.get_stored_dimension() {
                if stored_dim != provider_dim {
                    eprintln!(
                        "⚠️  WARNING: Stored vectors have dimension {} but current embedding provider reports {}. Run `reindex` to re-embed all entries.",
                        stored_dim, provider_dim
                    );
                }
            }
        }

        Ok(Self {
            add_intel_uc: AddIntelUseCase::new(
                intel_repo.clone(),
                embedder.clone(),
                vector_store.clone(),
            ),
            search_uc: SearchUseCase::new(
                intel_repo.clone(),
                embedder.clone(),
                vector_store.clone(),
            ),
            query_uc: QueryUseCase::new(intel_repo.clone()),
            trade_uc: TradeUseCase::new(trade_repo),
            stats_uc: StatsUseCase::new(intel_repo.clone()),
            reindex_uc: ReindexUseCase::new(intel_repo, embedder, vector_store),
        })
    }

    // Delegating methods
    #[allow(clippy::too_many_arguments)]
    pub async fn add_intel(
        &self,
        category: Category,
        title: String,
        body: String,
        source: Option<String>,
        tags: Vec<String>,
        confidence: Option<f64>,
        actionable: Option<bool>,
        source_type: SourceType,
        metadata: Option<serde_json::Value>,
        skip_dedup: bool,
    ) -> Result<AddResult, DomainError> {
        self.add_intel_uc
            .execute(
                category,
                title,
                body,
                source,
                tags,
                confidence,
                actionable,
                source_type,
                metadata,
                skip_dedup,
            )
            .await
    }

    pub fn keyword_search(&self, text: &str, limit: usize) -> Result<Vec<IntelEntry>, DomainError> {
        self.search_uc.keyword_search(text, limit)
    }

    pub fn keyword_search_with_time(
        &self,
        text: &str,
        limit: usize,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
    ) -> Result<Vec<IntelEntry>, DomainError> {
        self.search_uc
            .keyword_search_with_time(text, limit, since, until)
    }

    pub async fn semantic_search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<IntelEntry>, DomainError> {
        self.search_uc.semantic_search(query, limit).await
    }

    pub async fn hybrid_search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<IntelEntry>, DomainError> {
        self.search_uc.hybrid_search(query, limit).await
    }

    pub fn query(
        &self,
        category: Option<Category>,
        tag: Option<String>,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
        limit: Option<usize>,
        exclude_source_type: Option<SourceType>,
    ) -> Result<Vec<IntelEntry>, DomainError> {
        self.query_uc
            .execute(category, tag, since, until, limit, exclude_source_type)
    }

    pub fn trade_add(
        &self,
        ticker: String,
        series_ticker: Option<String>,
        direction: TradeDirection,
        contracts: i64,
        entry_price: f64,
        thesis: Option<String>,
    ) -> Result<Trade, DomainError> {
        self.trade_uc.add(
            ticker,
            series_ticker,
            direction,
            contracts,
            entry_price,
            thesis,
        )
    }

    pub fn trade_resolve(
        &self,
        id: &str,
        outcome: TradeOutcome,
        pnl_cents: i64,
        exit_price: Option<f64>,
    ) -> Result<(), DomainError> {
        self.trade_uc.resolve(id, outcome, pnl_cents, exit_price)
    }

    pub fn trade_list(
        &self,
        limit: Option<usize>,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
        resolved: Option<bool>,
    ) -> Result<Vec<Trade>, DomainError> {
        self.trade_uc.list(limit, since, until, resolved)
    }

    pub fn stats(&self) -> Result<IntelStats, DomainError> {
        self.stats_uc.stats()
    }

    pub fn tags(&self, category: Option<Category>) -> Result<Vec<TagCount>, DomainError> {
        self.stats_uc.tags(category)
    }

    pub async fn reindex(&self) -> Result<usize, DomainError> {
        self.reindex_uc.execute().await
    }
}
