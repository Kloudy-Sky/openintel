//! Opportunities use case — runs all registered strategies and returns
//! ranked opportunities.

use std::sync::Arc;

use chrono::{Duration, Utc};
use serde::Serialize;

use crate::domain::error::DomainError;
use crate::domain::ports::intel_repository::{IntelRepository, QueryFilter};
use crate::domain::ports::strategy::{DetectionContext, Opportunity, Strategy};
use crate::domain::ports::trade_repository::{TradeFilter, TradeRepository};

/// Result of running all strategies.
#[derive(Debug, Serialize)]
pub struct OpportunityScan {
    pub scanned_at: chrono::DateTime<chrono::Utc>,
    pub window_hours: u32,
    pub entries_scanned: usize,
    pub strategies_run: usize,
    pub strategies_failed: usize,
    pub total_opportunities: usize,
    pub opportunities: Vec<Opportunity>,
}

pub struct OpportunitiesUseCase {
    intel_repo: Arc<dyn IntelRepository>,
    trade_repo: Arc<dyn TradeRepository>,
    strategies: Vec<Box<dyn Strategy>>,
}

impl OpportunitiesUseCase {
    pub fn new(
        intel_repo: Arc<dyn IntelRepository>,
        trade_repo: Arc<dyn TradeRepository>,
        strategies: Vec<Box<dyn Strategy>>,
    ) -> Self {
        Self {
            intel_repo,
            trade_repo,
            strategies,
        }
    }

    /// Run all strategies and return ranked opportunities.
    ///
    /// `entry_limit` caps how many recent entries to load (default 500).
    /// `result_limit` caps how many opportunities to return (default unlimited).
    pub fn execute(
        &self,
        window_hours: u32,
        min_score: Option<f64>,
        entry_limit: Option<usize>,
        result_limit: Option<usize>,
    ) -> Result<OpportunityScan, DomainError> {
        let now = Utc::now();
        let since = now - Duration::hours(window_hours as i64);

        let limit = entry_limit.unwrap_or(500);

        // Fetch recent entries
        let filter = QueryFilter {
            since: Some(since),
            limit: Some(limit),
            ..Default::default()
        };
        let entries = self.intel_repo.query(&filter)?;
        let entries_scanned = entries.len();

        // Fetch open trades
        let trade_filter = TradeFilter {
            limit: Some(100),
            resolved: Some(false),
            ..Default::default()
        };
        let open_trades = self.trade_repo.list_trades(&trade_filter)?;

        let ctx = DetectionContext {
            entries,
            open_trades,
            window_hours,
        };

        let mut all_opportunities = Vec::new();
        let mut strategies_succeeded = 0usize;

        for strategy in &self.strategies {
            match strategy.detect(&ctx) {
                Ok(mut opps) => {
                    strategies_succeeded += 1;
                    all_opportunities.append(&mut opps);
                }
                Err(e) => {
                    eprintln!("WARNING: Strategy '{}' failed: {}", strategy.name(), e);
                }
            }
        }

        // Apply minimum score filter
        if let Some(min) = min_score {
            all_opportunities.retain(|o| o.score >= min);
        }

        // Sort by score descending, then by title for deterministic ordering
        all_opportunities.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.title.cmp(&b.title))
        });

        // Apply result limit
        if let Some(max) = result_limit {
            all_opportunities.truncate(max);
        }

        Ok(OpportunityScan {
            scanned_at: now,
            window_hours,
            entries_scanned,
            strategies_run: strategies_succeeded,
            strategies_failed: self.strategies.len() - strategies_succeeded,
            total_opportunities: all_opportunities.len(),
            opportunities: all_opportunities,
        })
    }
}
