//! IO edge for `option`: fetch the underlying's bars and snapshot, then run
//! the pure defined-risk frame. The premium comes from the caller (the
//! broker's chain quote is live; a keyless chain would be stale), the clock
//! is injected, and a missing snapshot becomes a note, never a zero.

use chrono::{DateTime, Utc};

use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::option::{option_frame as frame, MarketContext, OptionFrame, OptionSpec};
use crate::domain::ports::bar_source::BarSource;
use crate::domain::ports::market_data_source::MarketDataSource;
use crate::domain::values::asset_class::AssetClass;

pub const FRAMING: &str = "option_frame is a calculator, not advice — it sizes a long option to a \
     budget and states the move it needs; it never rates the odds of getting it.";

pub async fn option_frame(
    spec: OptionSpec,
    bars: &dyn BarSource,
    market: &dyn MarketDataSource,
    now: DateTime<Utc>,
) -> Result<OptionFrame, DomainError> {
    let ticker = Ticker::parse(&spec.underlying)?;
    if ticker.class() != AssetClass::Equity {
        return Err(DomainError::SourceFailure {
            name: "option".into(),
            message: format!(
                "{} is {}; listed options here are framed on equities and ETFs only",
                ticker.as_str(),
                ticker.class().as_str()
            ),
        });
    }
    let spec = OptionSpec {
        underlying: ticker.as_str().to_string(),
        ..spec
    };
    let history = bars.bars(&ticker).await?;
    let mut extra_notes = Vec::new();
    let ctx = match market.snapshot(&ticker).await {
        Ok(snap) => MarketContext {
            spot: Some(snap.last_price),
            realized_vol: snap.realized_vol,
            iv_rank: snap.iv_rank,
        },
        Err(e) => {
            extra_notes.push(format!(
                "snapshot unavailable ({e}); spot taken from the last daily close"
            ));
            MarketContext {
                spot: history.last().map(|b| b.close),
                realized_vol: None,
                iv_rank: None,
            }
        }
    };
    let mut out = frame(&spec, ctx, &history, now)?;
    out.notes.extend(extra_notes);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::market::mock_market::MockMarketSource;
    use crate::domain::trade_journal::OptionKind;
    use crate::domain::values::bar::Bar;
    use async_trait::async_trait;
    use chrono::{NaiveDate, TimeZone};

    struct FixedBars(Vec<Bar>);

    #[async_trait]
    impl BarSource for FixedBars {
        async fn bars(&self, _t: &Ticker) -> Result<Vec<Bar>, DomainError> {
            Ok(self.0.clone())
        }
    }

    fn history() -> Vec<Bar> {
        (0..16)
            .map(|_| Bar {
                date: NaiveDate::from_ymd_opt(2026, 9, 4).unwrap(),
                open: 190.0,
                high: 194.0,
                low: 186.0,
                close: 190.0,
            })
            .collect()
    }

    fn spec(underlying: &str) -> OptionSpec {
        OptionSpec {
            underlying: underlying.into(),
            kind: OptionKind::Call,
            strike: 200.0,
            expiry: NaiveDate::from_ymd_opt(2026, 10, 16).unwrap(),
            premium: 4.0,
            budget_usd: 1000.0,
        }
    }

    #[tokio::test]
    async fn frames_an_equity_from_snapshot_spot_and_bars() {
        let f = option_frame(
            spec("aapl"),
            &FixedBars(history()),
            &MockMarketSource,
            Utc.with_ymd_and_hms(2026, 9, 8, 14, 0, 0).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(f.underlying, "AAPL");
        assert_eq!(f.contracts, 2);
        assert_eq!(f.spot, Some(192.5));
        assert_eq!(f.iv_rank, Some(0.82));
        assert!(f.atr.is_some());
    }

    #[tokio::test]
    async fn refuses_non_equity_underlyings() {
        let err = option_frame(
            spec("BTC-USD"),
            &FixedBars(history()),
            &MockMarketSource,
            Utc::now(),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("equities and ETFs only"));
    }
}
