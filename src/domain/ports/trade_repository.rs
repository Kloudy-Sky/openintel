use crate::domain::entities::trade::Trade;
use crate::domain::values::trade_outcome::TradeOutcome;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Default)]
pub struct TradeFilter {
    pub limit: Option<usize>,
    pub since: Option<DateTime<Utc>>,
    pub resolved: Option<bool>,
}

pub trait TradeRepository: Send + Sync {
    fn add_trade(&self, trade: &Trade) -> Result<(), String>;
    fn resolve_trade(&self, id: &str, outcome: TradeOutcome, pnl_cents: i64, exit_price: Option<f64>) -> Result<(), String>;
    fn list_trades(&self, filter: &TradeFilter) -> Result<Vec<Trade>, String>;
    fn get_trade(&self, id: &str) -> Result<Option<Trade>, String>;
}
