use crate::domain::entities::trade::Trade;
use crate::domain::ports::trade_repository::{TradeFilter, TradeRepository};
use crate::domain::values::trade_direction::TradeDirection;
use crate::domain::values::trade_outcome::TradeOutcome;
use chrono::{DateTime, Utc};
use std::sync::Arc;

pub struct TradeUseCase {
    repo: Arc<dyn TradeRepository>,
}

impl TradeUseCase {
    pub fn new(repo: Arc<dyn TradeRepository>) -> Self {
        Self { repo }
    }

    pub fn add(
        &self,
        ticker: String,
        series_ticker: Option<String>,
        direction: TradeDirection,
        contracts: i64,
        entry_price: f64,
        thesis: Option<String>,
    ) -> Result<Trade, String> {
        let trade = Trade::new(ticker, series_ticker, direction, contracts, entry_price, thesis);
        self.repo.add_trade(&trade)?;
        Ok(trade)
    }

    pub fn resolve(&self, id: &str, outcome: TradeOutcome, pnl_cents: i64, exit_price: Option<f64>) -> Result<(), String> {
        self.repo.resolve_trade(id, outcome, pnl_cents, exit_price)
    }

    pub fn list(
        &self,
        limit: Option<usize>,
        since: Option<DateTime<Utc>>,
        resolved: Option<bool>,
    ) -> Result<Vec<Trade>, String> {
        self.repo.list_trades(&TradeFilter { limit, since, resolved })
    }
}
