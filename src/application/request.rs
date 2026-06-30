use crate::domain::engine::config::EngineConfig;
use crate::domain::values::source_kind::SourceKind;

/// Presentation-free input to the analysis use case. Carries only analysis
/// parameters — no output format or rendering concerns (those belong to the
/// driving adapter: CLI or MCP).
#[derive(Debug, Clone)]
pub struct AnalysisRequest {
    pub ticker: String,
    pub enabled_sources: Vec<SourceKind>,
    pub market_enabled: bool,
    pub limit: usize,
    pub engine: EngineConfig,
}
