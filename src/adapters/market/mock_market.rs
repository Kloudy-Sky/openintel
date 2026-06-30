use async_trait::async_trait;
use chrono::{TimeZone, Utc};

use crate::domain::entities::market_snapshot::MarketSnapshot;
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::ports::market_data_source::MarketDataSource;

pub struct MockMarketSource;

#[async_trait]
impl MarketDataSource for MockMarketSource {
    fn name(&self) -> &'static str {
        "mock-market"
    }

    async fn snapshot(&self, ticker: &Ticker) -> Result<MarketSnapshot, DomainError> {
        Ok(MarketSnapshot {
            ticker: ticker.clone(),
            as_of: Utc.with_ymd_and_hms(2026, 6, 24, 20, 0, 0).unwrap(),
            last_price: 192.50,
            previous_close: 185.00,
            volume: 95_000_000,
            avg_volume: 52_000_000,
            realized_vol: Some(0.38),
            put_call_ratio: Some(0.7),
            iv_rank: Some(0.82),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn returns_fixture_snapshot() {
        let snap = MockMarketSource
            .snapshot(&Ticker::parse("AAPL").unwrap())
            .await
            .unwrap();
        assert_eq!(snap.last_price, 192.50);
        assert_eq!(snap.iv_rank, Some(0.82));
        assert_eq!(MockMarketSource.name(), "mock-market");
    }
}
