use crate::domain::error::DomainError;
use crate::domain::ports::resolution_source::ResolutionSource;
use crate::domain::ports::trade_repository::{TradeFilter, TradeRepository};
use serde::Serialize;
use std::sync::Arc;

pub struct ResolveTradesUseCase {
    trade_repo: Arc<dyn TradeRepository>,
}

#[derive(Debug, Serialize)]
pub struct ResolveReport {
    pub checked: usize,
    pub resolved: Vec<ResolvedTrade>,
    pub unresolved: Vec<UnresolvedTrade>,
    pub errors: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ResolvedTrade {
    pub trade_id: String,
    pub ticker: String,
    pub outcome: String,
    pub pnl_cents: i64,
    pub source: String,
    pub reason: String,
}

#[derive(Debug, Serialize)]
pub struct UnresolvedTrade {
    pub trade_id: String,
    pub ticker: String,
    pub direction: String,
    pub entry_price: f64,
    pub contracts: i64,
    pub age_hours: f64,
}

impl ResolveTradesUseCase {
    pub fn new(trade_repo: Arc<dyn TradeRepository>) -> Self {
        Self { trade_repo }
    }

    /// Check all open trades against resolution sources and auto-resolve where possible.
    pub async fn execute(
        &self,
        sources: &[Arc<dyn ResolutionSource>],
    ) -> Result<ResolveReport, DomainError> {
        let open_trades = self.trade_repo.list_trades(&TradeFilter {
            limit: None,
            since: None,
            resolved: Some(false),
        })?;

        let checked = open_trades.len();
        let mut resolved = Vec::new();
        let mut unresolved = Vec::new();
        let mut errors = Vec::new();

        for trade in &open_trades {
            let mut was_resolved = false;

            for source in sources {
                match source.check(trade).await {
                    Ok(Some(result)) if result.resolved => {
                        // Auto-resolve the trade
                        let outcome = result.outcome.unwrap_or(
                            crate::domain::values::trade_outcome::TradeOutcome::Scratch,
                        );
                        let pnl = result.pnl_cents.unwrap_or(0);

                        if let Err(e) = self.trade_repo.resolve_trade(
                            &trade.id,
                            outcome,
                            pnl,
                            result.exit_price,
                        ) {
                            errors.push(format!(
                                "Failed to resolve trade {}: {}",
                                trade.id, e
                            ));
                            continue;
                        }

                        resolved.push(ResolvedTrade {
                            trade_id: trade.id.clone(),
                            ticker: trade.ticker.clone(),
                            outcome: outcome.to_string(),
                            pnl_cents: pnl,
                            source: source.name().to_string(),
                            reason: result.reason,
                        });
                        was_resolved = true;
                        break;
                    }
                    Ok(_) => {} // Not resolved by this source
                    Err(e) => {
                        errors.push(format!(
                            "Error checking trade {} via {}: {}",
                            trade.id,
                            source.name(),
                            e
                        ));
                    }
                }
            }

            if !was_resolved {
                let age = (chrono::Utc::now() - trade.created_at).num_minutes() as f64 / 60.0;
                unresolved.push(UnresolvedTrade {
                    trade_id: trade.id.clone(),
                    ticker: trade.ticker.clone(),
                    direction: trade.direction.to_string(),
                    entry_price: trade.entry_price,
                    contracts: trade.contracts,
                    age_hours: (age * 10.0).round() / 10.0,
                });
            }
        }

        Ok(ResolveReport {
            checked,
            resolved,
            unresolved,
            errors,
        })
    }

    /// List open trades without attempting resolution (dry-run / status check).
    pub fn pending(&self) -> Result<ResolveReport, DomainError> {
        let open_trades = self.trade_repo.list_trades(&TradeFilter {
            limit: None,
            since: None,
            resolved: Some(false),
        })?;

        let checked = open_trades.len();
        let unresolved: Vec<UnresolvedTrade> = open_trades
            .iter()
            .map(|trade| {
                let age = (chrono::Utc::now() - trade.created_at).num_minutes() as f64 / 60.0;
                UnresolvedTrade {
                    trade_id: trade.id.clone(),
                    ticker: trade.ticker.clone(),
                    direction: trade.direction.to_string(),
                    entry_price: trade.entry_price,
                    contracts: trade.contracts,
                    age_hours: (age * 10.0).round() / 10.0,
                }
            })
            .collect();

        Ok(ResolveReport {
            checked,
            resolved: Vec::new(),
            unresolved,
            errors: Vec::new(),
        })
    }
}
