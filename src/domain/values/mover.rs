/// One row of a "biggest movers" screener. Optional fields are optional
/// because screeners omit them for some instruments — the quality floor
/// treats a missing value as a reject, never a pass.
#[derive(Debug, Clone, PartialEq)]
pub struct MoverRow {
    pub symbol: String,
    /// Regular-session change in percent (e.g. -11.2 for an 11.2% drop).
    pub change_pct: f64,
    pub price: f64,
    pub market_cap: Option<u64>,
    pub avg_volume_3mo: Option<u64>,
    pub day_volume: Option<u64>,
    pub exchange: String,
    /// First trade timestamp in epoch milliseconds (listing-age screen).
    pub first_trade_ms: Option<i64>,
}
