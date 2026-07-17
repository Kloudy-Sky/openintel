use chrono::{DateTime, Utc};

use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::ports::bar_source::BarSource;
use crate::domain::risk::{frame, Direction, RiskFrame};

pub const DEFAULT_STOP_MULTIPLE: f64 = 2.0;

/// Fetch bars, default the entry to the last close, run the pure frame math.
/// Clock injected at this edge; all validation errors are clean messages.
pub async fn risk_frame(
    ticker_raw: &str,
    direction: Direction,
    budget_usd: f64,
    stop_multiple: Option<f64>,
    entry: Option<f64>,
    bars: &dyn BarSource,
    now: DateTime<Utc>,
) -> Result<RiskFrame, DomainError> {
    let ticker = Ticker::parse(ticker_raw)?;
    let history = bars.bars(&ticker).await?;
    let entry = match entry {
        Some(e) => e,
        None => {
            history
                .last()
                .ok_or_else(|| DomainError::SourceFailure {
                    name: "risk".into(),
                    message: "no price history".into(),
                })?
                .close
        }
    };
    frame(
        ticker.as_str(),
        &history,
        direction,
        entry,
        budget_usd,
        stop_multiple.unwrap_or(DEFAULT_STOP_MULTIPLE),
        now,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::values::bar::Bar;
    use async_trait::async_trait;
    use chrono::TimeZone;

    fn at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 16, 0, 0, 0).unwrap()
    }

    struct FixedBars(Vec<Bar>);

    #[async_trait]
    impl BarSource for FixedBars {
        async fn bars(&self, _t: &Ticker) -> Result<Vec<Bar>, DomainError> {
            Ok(self.0.clone())
        }
    }

    fn history() -> Vec<Bar> {
        let mut v = vec![Bar {
            high: 101.0,
            low: 99.0,
            close: 100.0,
        }];
        for _ in 0..15 {
            v.push(Bar {
                high: 108.0,
                low: 104.0,
                close: 106.0,
            });
        }
        v
    }

    #[tokio::test]
    async fn defaults_entry_to_last_close_and_multiple_to_two() {
        let f = risk_frame(
            "nvda",
            Direction::Long,
            200.0,
            None,
            None,
            &FixedBars(history()),
            at(),
        )
        .await
        .unwrap();
        assert_eq!(f.ticker, "NVDA");
        assert!((f.entry - 106.0).abs() < 1e-12);
        assert!((f.stop_multiple - 2.0).abs() < 1e-12);
        assert_eq!(f.generated_at, at());
    }

    #[tokio::test]
    async fn entry_override_and_errors_propagate() {
        let f = risk_frame(
            "NVDA",
            Direction::Short,
            100.0,
            Some(1.0),
            Some(110.0),
            &FixedBars(history()),
            at(),
        )
        .await
        .unwrap();
        assert!((f.entry - 110.0).abs() < 1e-12);
        assert!(risk_frame(
            "$$$",
            Direction::Long,
            100.0,
            None,
            None,
            &FixedBars(history()),
            at()
        )
        .await
        .is_err());
        assert!(risk_frame(
            "NVDA",
            Direction::Long,
            100.0,
            None,
            None,
            &FixedBars(vec![]),
            at()
        )
        .await
        .is_err());
    }
}
