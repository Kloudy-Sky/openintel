//! Parser for Yahoo's predefined `day_losers` screener (keyless, unofficial —
//! failure mode is a clean error, never a partial silent result).

use serde::Deserialize;

use crate::domain::error::DomainError;
use crate::domain::values::mover::MoverRow;

#[derive(Debug, Deserialize)]
struct ScreenerResponse {
    finance: Finance,
}

#[derive(Debug, Deserialize)]
struct Finance {
    #[serde(default)]
    result: Option<Vec<ScreenerResult>>,
    #[serde(default)]
    error: Option<FinanceError>,
}

#[derive(Debug, Deserialize)]
struct FinanceError {
    #[serde(default)]
    code: String,
    #[serde(default)]
    description: String,
}

#[derive(Debug, Deserialize)]
struct ScreenerResult {
    #[serde(default)]
    quotes: Vec<ScreenerQuote>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScreenerQuote {
    #[serde(default)]
    symbol: Option<String>,
    #[serde(default)]
    regular_market_change_percent: Option<f64>,
    #[serde(default)]
    regular_market_price: Option<f64>,
    #[serde(default)]
    market_cap: Option<u64>,
    #[serde(default)]
    average_daily_volume3_month: Option<u64>,
    #[serde(default)]
    regular_market_volume: Option<u64>,
    #[serde(default)]
    full_exchange_name: Option<String>,
    #[serde(default)]
    first_trade_date_milliseconds: Option<i64>,
}

fn fail(message: impl Into<String>) -> DomainError {
    DomainError::SourceFailure {
        name: "yahoo-screener".into(),
        message: message.into(),
    }
}

/// Parse the screener body into rows plus a count of rows skipped for missing
/// required fields (symbol, change, price, exchange). Optional fields stay
/// optional — the quality floor decides what a missing value means.
pub(crate) fn parse_movers(body: &str) -> Result<(Vec<MoverRow>, usize), DomainError> {
    let resp: ScreenerResponse =
        serde_json::from_str(body).map_err(|e| fail(format!("malformed response: {e}")))?;
    if let Some(err) = resp.finance.error {
        return Err(fail(format!("{}: {}", err.code, err.description)));
    }
    let quotes = resp
        .finance
        .result
        .and_then(|mut r| (!r.is_empty()).then(|| r.remove(0)))
        .ok_or_else(|| fail("empty result"))?
        .quotes;

    let mut rows = Vec::new();
    let mut skipped = 0usize;
    for q in quotes {
        match (
            q.symbol,
            q.regular_market_change_percent,
            q.regular_market_price,
            q.full_exchange_name,
        ) {
            (Some(symbol), Some(change_pct), Some(price), Some(exchange)) => {
                rows.push(MoverRow {
                    symbol,
                    change_pct,
                    price,
                    market_cap: q.market_cap,
                    avg_volume_3mo: q.average_daily_volume3_month,
                    day_volume: q.regular_market_volume,
                    exchange,
                    first_trade_ms: q.first_trade_date_milliseconds,
                });
            }
            _ => skipped += 1,
        }
    }
    if rows.is_empty() {
        return Err(fail("screener returned no usable rows"));
    }
    Ok((rows, skipped))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HAPPY: &str = r#"{"finance":{"result":[{"quotes":[
        {"symbol":"BLSH","regularMarketChangePercent":-11.24,"regularMarketPrice":24.39,
         "marketCap":3698703104,"averageDailyVolume3Month":1607109,"regularMarketVolume":3086768,
         "fullExchangeName":"NYSE","firstTradeDateMilliseconds":1755091800000},
        {"symbol":"NOCAP","regularMarketChangePercent":-9.0,"regularMarketPrice":12.0,
         "fullExchangeName":"NasdaqGS"},
        {"regularMarketChangePercent":-8.0,"regularMarketPrice":5.0,"fullExchangeName":"NYSE"}
    ]}],"error":null}}"#;

    #[test]
    fn parses_rows_and_skips_malformed() {
        let (rows, skipped) = parse_movers(HAPPY).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(skipped, 1); // the symbol-less row
        assert_eq!(rows[0].symbol, "BLSH");
        assert!((rows[0].change_pct + 11.24).abs() < 1e-9);
        assert_eq!(rows[0].market_cap, Some(3698703104));
        assert_eq!(rows[0].first_trade_ms, Some(1755091800000));
        // optional fields survive as None
        assert_eq!(rows[1].market_cap, None);
        assert_eq!(rows[1].avg_volume_3mo, None);
    }

    #[test]
    fn error_body_empty_result_and_no_rows_fail() {
        let err = r#"{"finance":{"result":null,"error":{"code":"Bad","description":"nope"}}}"#;
        assert!(parse_movers(err).is_err());
        assert!(parse_movers(r#"{"finance":{"result":[],"error":null}}"#).is_err());
        // rows present but all malformed -> error, not silent empty
        let all_bad = r#"{"finance":{"result":[{"quotes":[{"symbol":"X"}]}],"error":null}}"#;
        assert!(parse_movers(all_bad).is_err());
        assert!(parse_movers("not json").is_err());
    }
}
