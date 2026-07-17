/// One daily OHLC bar (open omitted — nothing here needs it).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bar {
    pub high: f64,
    pub low: f64,
    pub close: f64,
}
