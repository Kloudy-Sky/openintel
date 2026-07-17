use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;

use crate::domain::entities::market_snapshot::MarketSnapshot;
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::values::bar::Bar;

const MIN_RETURNS_FOR_VOL: usize = 20;
const TRADING_DAYS: f64 = 252.0;

#[derive(Debug, Deserialize)]
struct ChartResponse {
    chart: Chart,
}

#[derive(Debug, Deserialize)]
struct Chart {
    #[serde(default)]
    result: Option<Vec<ChartResult>>,
    #[serde(default)]
    error: Option<YahooError>,
}

#[derive(Debug, Deserialize)]
struct YahooError {
    #[serde(default)]
    code: String,
    #[serde(default)]
    description: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChartResult {
    meta: Meta,
    #[serde(default)]
    timestamp: Option<Vec<i64>>,
    indicators: Indicators,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Meta {
    #[serde(default)]
    regular_market_price: Option<f64>,
    #[serde(default)]
    chart_previous_close: Option<f64>,
    #[serde(default)]
    regular_market_volume: Option<u64>,
    #[serde(default)]
    regular_market_time: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct Indicators {
    quote: Vec<Quote>,
}

#[derive(Debug, Deserialize)]
struct Quote {
    #[serde(default)]
    close: Vec<Option<f64>>,
    #[serde(default)]
    volume: Vec<Option<u64>>,
    #[serde(default)]
    high: Vec<Option<f64>>,
    #[serde(default)]
    low: Vec<Option<f64>>,
}

fn fail(message: impl Into<String>) -> DomainError {
    DomainError::SourceFailure {
        name: "yahoo".into(),
        message: message.into(),
    }
}

/// From a parsed `ChartResponse`, surface a JSON-level Yahoo error (e.g. a
/// delisted ticker) or the first (and only) chart result.
fn extract_result(resp: ChartResponse) -> Result<ChartResult, DomainError> {
    if let Some(err) = resp.chart.error {
        return Err(fail(format!("{}: {}", err.code, err.description)));
    }
    resp.chart
        .result
        .and_then(|mut r| (!r.is_empty()).then(|| r.remove(0)))
        .ok_or_else(|| fail("empty result"))
}

/// Shared by `parse_snapshot` and `parse_bars`: both start from a
/// `ChartResult` and need only its first (and only) quote series.
fn extract_quote(indicators: Indicators) -> Result<Quote, DomainError> {
    indicators
        .quote
        .into_iter()
        .next()
        .ok_or_else(|| fail("no quote series"))
}

pub(crate) fn sample_stdev(xs: &[f64]) -> Option<f64> {
    if xs.len() < 2 {
        return None;
    }
    let n = xs.len() as f64;
    let mean = xs.iter().sum::<f64>() / n;
    let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0);
    Some(var.sqrt())
}

pub(crate) fn log_returns(closes: &[f64]) -> Vec<f64> {
    closes.windows(2).map(|w| (w[1] / w[0]).ln()).collect()
}

pub(crate) fn realized_vol(closes: &[f64], min_returns: usize) -> Option<f64> {
    let returns = log_returns(closes);
    if returns.len() < min_returns {
        return None;
    }
    sample_stdev(&returns).map(|s| s * TRADING_DAYS.sqrt())
}

pub(crate) fn parse_snapshot(
    body: &str,
    ticker: &Ticker,
    fetched_at: DateTime<Utc>,
) -> Result<MarketSnapshot, DomainError> {
    let resp: ChartResponse =
        serde_json::from_str(body).map_err(|e| fail(format!("malformed response: {e}")))?;

    let result = extract_result(resp)?;
    let meta = result.meta;
    let timestamp = result.timestamp;
    let quote = extract_quote(result.indicators)?;

    let closes: Vec<f64> = quote.close.into_iter().flatten().collect();
    let volumes: Vec<u64> = quote.volume.into_iter().flatten().collect();

    let last_price = meta
        .regular_market_price
        .or_else(|| closes.last().copied())
        .ok_or_else(|| fail("no last price"))?;

    let previous_close = closes
        .len()
        .checked_sub(2)
        .and_then(|i| closes.get(i).copied())
        .or(meta.chart_previous_close)
        .ok_or_else(|| fail("no previous close"))?;

    let volume = meta
        .regular_market_volume
        .or_else(|| volumes.last().copied())
        .unwrap_or(0);

    let avg_volume = if volumes.is_empty() {
        0
    } else {
        (volumes.iter().sum::<u64>() as f64 / volumes.len() as f64).round() as u64
    };

    let realized_vol = realized_vol(&closes, MIN_RETURNS_FOR_VOL);

    let as_of = meta
        .regular_market_time
        .or_else(|| timestamp.as_ref().and_then(|t| t.last().copied()))
        .and_then(|secs| Utc.timestamp_opt(secs, 0).single())
        .unwrap_or(fetched_at);

    Ok(MarketSnapshot {
        ticker: ticker.clone(),
        as_of,
        last_price,
        previous_close,
        volume,
        avg_volume,
        realized_vol,
        put_call_ratio: None,
        iv_rank: None,
    })
}

/// OHLC bars from the same chart response `parse_snapshot` reads. Rows with
/// any missing leg (Yahoo emits nulls for halts/partial days) are skipped.
pub(crate) fn parse_bars(body: &str) -> Result<Vec<Bar>, DomainError> {
    let chart: ChartResponse =
        serde_json::from_str(body).map_err(|e| fail(format!("malformed response: {e}")))?;
    let result = extract_result(chart)?;
    let quote = extract_quote(result.indicators)?;
    let bars = quote
        .high
        .iter()
        .zip(quote.low.iter())
        .zip(quote.close.iter())
        .filter_map(|((h, l), c)| {
            Some(Bar {
                high: (*h)?,
                low: (*l)?,
                close: (*c)?,
            })
        })
        .collect();
    Ok(bars)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn tkr() -> Ticker {
        Ticker::parse("AAPL").unwrap()
    }
    fn at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 30, 0, 0, 0).unwrap()
    }

    // 3 daily bars; last live price in meta differs from series last close.
    const HAPPY: &str = r#"{"chart":{"result":[{
        "meta":{"regularMarketPrice":192.5,"chartPreviousClose":170.0,
                "regularMarketVolume":95000000,"regularMarketTime":1782504000},
        "timestamp":[1782327600,1782414000,1782500400],
        "indicators":{"quote":[{"close":[185.0,188.0,191.0],"volume":[50000000,60000000,95000000]}]}
    }],"error":null}}"#;

    const NULL_PADDED: &str = r#"{"chart":{"result":[{
        "meta":{"regularMarketPrice":10.0,"regularMarketVolume":30},
        "timestamp":[1,2,3,4],
        "indicators":{"quote":[{"close":[null,8.0,null,9.0],"volume":[null,10,null,20]}]}
    }],"error":null}}"#;

    const ERROR_BODY: &str = r#"{"chart":{"result":null,"error":{"code":"Not Found","description":"No data found, symbol may be delisted"}}}"#;

    const EMPTY_RESULT: &str = r#"{"chart":{"result":[],"error":null}}"#;

    const NO_PRICE: &str = r#"{"chart":{"result":[{
        "meta":{},"timestamp":[],"indicators":{"quote":[{"close":[],"volume":[]}]}
    }],"error":null}}"#;

    #[test]
    fn happy_path_maps_all_fields() {
        let s = parse_snapshot(HAPPY, &tkr(), at()).unwrap();
        assert_eq!(s.ticker.as_str(), "AAPL");
        assert_eq!(s.last_price, 192.5); // from meta.regularMarketPrice
        assert_eq!(s.previous_close, 188.0); // 2nd-to-last non-null close
        assert_eq!(s.volume, 95000000); // meta.regularMarketVolume
        assert_eq!(s.avg_volume, 68333333); // round((50+60+95)e6 / 3)
        assert_eq!(s.realized_vol, None); // only 2 returns < 20
        assert_eq!(s.put_call_ratio, None);
        assert_eq!(s.iv_rank, None);
        assert_eq!(s.as_of, Utc.timestamp_opt(1782504000, 0).single().unwrap());
    }

    #[test]
    fn null_padding_is_dropped_order_preserved() {
        let s = parse_snapshot(NULL_PADDED, &tkr(), at()).unwrap();
        // non-null closes = [8.0, 9.0] -> previous_close = 8.0
        assert_eq!(s.previous_close, 8.0);
        assert_eq!(s.last_price, 10.0); // meta price
                                        // non-null volumes = [10, 20] -> avg = 15
        assert_eq!(s.avg_volume, 15);
        assert_eq!(s.volume, 30); // meta volume
                                  // no meta time, no fallback timestamp path returns last timestamp = 4
        assert_eq!(s.as_of, Utc.timestamp_opt(4, 0).single().unwrap());
    }

    #[test]
    fn chart_error_is_source_failure() {
        let err = parse_snapshot(ERROR_BODY, &tkr(), at()).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("yahoo"), "got {msg}");
        assert!(msg.contains("delisted"), "got {msg}");
    }

    #[test]
    fn empty_result_is_source_failure() {
        assert!(parse_snapshot(EMPTY_RESULT, &tkr(), at()).is_err());
    }

    #[test]
    fn missing_price_is_source_failure() {
        assert!(parse_snapshot(NO_PRICE, &tkr(), at()).is_err());
    }

    #[test]
    fn malformed_json_is_source_failure() {
        assert!(parse_snapshot("not json", &tkr(), at()).is_err());
    }

    #[test]
    fn sample_stdev_math() {
        assert_eq!(sample_stdev(&[1.0, 2.0, 3.0]), Some(1.0)); // var=1, stdev=1
        assert_eq!(sample_stdev(&[2.0, 2.0]), Some(0.0));
        assert_eq!(sample_stdev(&[5.0]), None);
        assert_eq!(sample_stdev(&[]), None);
    }

    #[test]
    fn log_returns_len_and_values() {
        let r = log_returns(&[100.0, 110.0, 121.0]);
        assert_eq!(r.len(), 2);
        assert!((r[0] - 1.1f64.ln()).abs() < 1e-12);
        assert!((r[1] - 1.1f64.ln()).abs() < 1e-12);
    }

    #[test]
    fn parse_bars_zips_and_skips_null_legs() {
        let body = r#"{"chart":{"result":[{"meta":{},"indicators":{"quote":[{
            "close":[100.0,106.0,null,107.0],
            "volume":[1,1,1,1],
            "high":[101.0,108.0,109.0,null],
            "low":[99.0,104.0,105.0,106.0]
        }]}}],"error":null}}"#;
        let bars = parse_bars(body).unwrap();
        assert_eq!(bars.len(), 2); // rows 2 (null close) and 3 (null high) skipped
        assert_eq!(bars[0].high, 101.0);
        assert_eq!(bars[1].close, 106.0);
    }

    #[test]
    fn parse_bars_malformed_and_empty() {
        assert!(parse_bars("nope").is_err());
    }

    #[test]
    fn realized_vol_gate_and_value() {
        // gate: fewer than min_returns -> None
        assert_eq!(realized_vol(&[100.0, 110.0], 20), None);
        // equal returns -> stdev 0 -> Some(0.0)
        assert_eq!(realized_vol(&[100.0, 110.0, 121.0], 2), Some(0.0));
        // known value: closes [100,110,90], min 2 -> ~3.3223 (annualized, sqrt(252))
        let v = realized_vol(&[100.0, 110.0, 90.0], 2).unwrap();
        assert!((v - 3.3223).abs() < 1e-3, "got {v}");
    }
}
