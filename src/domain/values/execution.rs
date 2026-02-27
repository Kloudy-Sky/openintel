/// Domain types for automated execution pipeline

use serde::{Deserialize, Serialize};

/// A trade plan action ready for execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradePlan {
    pub ticker: String,
    pub direction: String,
    pub size_cents: u64,
    pub confidence: f64,
    pub score: f64,
    pub edge_cents: Option<f64>,
    pub action: String,
    pub description: String,
}

/// An opportunity that was filtered out during trade plan building
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkippedOpportunity {
    pub title: String,
    pub confidence: f64,
    pub score: f64,
    pub reason: String,
}

/// Result of an execution pipeline run
#[derive(Debug, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub timestamp: String,
    pub mode: ExecutionMode,
    pub bankroll_cents: u64,
    pub feeds_ingested: usize,
    pub feed_errors: Vec<String>,
    pub opportunities_scanned: usize,
    pub trades_qualified: usize,
    pub trades_skipped: usize,
    pub total_deployment_cents: u64,
    pub trades: Vec<TradePlan>,
    pub skipped: Vec<SkippedOpportunity>,
}

/// Execution mode for the automated pipeline
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    DryRun,
    Live,
}

impl std::fmt::Display for ExecutionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecutionMode::DryRun => write!(f, "dry_run"),
            ExecutionMode::Live => write!(f, "live"),
        }
    }
}
