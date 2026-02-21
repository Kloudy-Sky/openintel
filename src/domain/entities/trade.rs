use crate::domain::values::trade_direction::TradeDirection;
use crate::domain::values::trade_outcome::TradeOutcome;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trade {
    pub id: String,
    pub ticker: String,
    pub series_ticker: Option<String>,
    pub direction: TradeDirection,
    pub contracts: i64,
    pub entry_price: f64,
    pub exit_price: Option<f64>,
    pub thesis: Option<String>,
    pub outcome: Option<TradeOutcome>,
    pub pnl_cents: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
}

impl Trade {
    pub fn new(
        ticker: String,
        series_ticker: Option<String>,
        direction: TradeDirection,
        contracts: i64,
        entry_price: f64,
        thesis: Option<String>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            ticker,
            series_ticker,
            direction,
            contracts,
            entry_price,
            exit_price: None,
            thesis,
            outcome: None,
            pnl_cents: None,
            created_at: Utc::now(),
            resolved_at: None,
        }
    }

    pub fn resolve(&mut self, outcome: TradeOutcome, pnl_cents: i64, exit_price: Option<f64>) {
        self.outcome = Some(outcome);
        self.pnl_cents = Some(pnl_cents);
        self.exit_price = exit_price;
        self.resolved_at = Some(Utc::now());
    }

    pub fn is_resolved(&self) -> bool {
        self.outcome.is_some()
    }
}
