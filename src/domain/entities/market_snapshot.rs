use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::domain::entities::ticker::Ticker;

#[derive(Debug, Clone, Serialize)]
pub struct MarketSnapshot {
    pub ticker: Ticker,
    pub as_of: DateTime<Utc>,
    pub last_price: f64,
    pub previous_close: f64,
    pub volume: u64,
    pub avg_volume: u64,
    pub realized_vol: Option<f64>,
    pub put_call_ratio: Option<f64>,
    pub iv_rank: Option<f64>,
}
