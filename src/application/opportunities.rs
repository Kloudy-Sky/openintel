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
    pub strategies_run: usize,
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
    pub fn execute(
        &self,
        window_hours: u32,
        min_score: Option<f64>,
    ) -> Result<OpportunityScan, DomainError> {
        let since = Utc::now() - Duration::hours(window_hours as i64);

        // Fetch recent entries
        let filter = QueryFilter {
            since: Some(since),
            limit: Some(500),
            ..Default::default()
        };
        let entries = self.intel_repo.query(&filter)?;

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

        for strategy in &self.strategies {
            match strategy.detect(&ctx) {
                Ok(mut opps) => all_opportunities.append(&mut opps),
                Err(e) => {
                    eprintln!(
                        "WARNING: Strategy '{}' failed: {}",
                        strategy.name(),
                        e
                    );
                }
            }
        }

        // Apply minimum score filter
        if let Some(min) = min_score {
            all_opportunities.retain(|o| o.score >= min);
        }

        // Sort by score descending
        all_opportunities.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        Ok(OpportunityScan {
            scanned_at: Utc::now(),
            window_hours,
            strategies_run: self.strategies.len(),
            total_opportunities: all_opportunities.len(),
            opportunities: all_opportunities,
        })
    }
}
