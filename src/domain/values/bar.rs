use chrono::NaiveDate;

/// One daily OHLC bar. `date` is the exchange-local session date — adapters
/// convert venue timestamps; domain math never touches a timezone.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bar {
    pub date: NaiveDate,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
}
