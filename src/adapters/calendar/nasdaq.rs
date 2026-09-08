//! Nasdaq's earnings calendar: keyless, unofficial, and it refuses non-browser
//! user agents. Failure mode is a clean error; a day with no rows is an empty
//! list, which the caller reports as "nothing listed", never as silence.

use std::time::Duration;

use async_trait::async_trait;
use chrono::NaiveDate;
use serde_json::Value;

use crate::domain::error::DomainError;
use crate::domain::ports::earnings_calendar_source::EarningsCalendarSource;
use crate::domain::values::earnings_row::{EarningsRow, ReportSlot};

const URL: &str = "https://api.nasdaq.com/api/calendar/earnings";
const TIMEOUT_SECS: u64 = 10;
const USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0 Safari/537.36";

fn fail(message: impl Into<String>) -> DomainError {
    DomainError::SourceFailure {
        name: "nasdaq".into(),
        message: message.into(),
    }
}

#[derive(Clone)]
pub struct NasdaqCalendar {
    client: reqwest::Client,
}

impl NasdaqCalendar {
    pub fn new() -> Result<Self, DomainError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(TIMEOUT_SECS))
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| fail(format!("client build failed: {e}")))?;
        Ok(Self { client })
    }
}

#[async_trait]
impl EarningsCalendarSource for NasdaqCalendar {
    async fn on(&self, date: NaiveDate) -> Result<Vec<EarningsRow>, DomainError> {
        let resp = self
            .client
            .get(format!("{URL}?date={date}"))
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| fail(format!("request failed: {e}")))?;
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| fail(format!("reading body failed (HTTP {status}): {e}")))?;
        if !status.is_success() {
            return Err(fail(format!("HTTP {status}")));
        }
        parse_rows(&body)
    }
}

pub(crate) fn parse_rows(body: &str) -> Result<Vec<EarningsRow>, DomainError> {
    let v: Value = serde_json::from_str(body).map_err(|e| fail(format!("malformed JSON: {e}")))?;
    let data = v.get("data").ok_or_else(|| fail("no data field"))?;
    let rows = match data.get("rows") {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(rows) => rows
            .as_array()
            .ok_or_else(|| fail("rows is not an array"))?,
    };
    let mut out = Vec::new();
    let mut skipped = 0;
    for r in rows {
        let Some(symbol) = str_field(r, "symbol").filter(|s| !s.is_empty()) else {
            skipped += 1;
            continue;
        };
        out.push(EarningsRow {
            symbol,
            company: str_field(r, "name").unwrap_or_default(),
            slot: slot(str_field(r, "time").as_deref()),
            market_cap_usd: str_field(r, "marketCap").and_then(|s| money(&s)),
            eps_forecast: str_field(r, "epsForecast").and_then(|s| money(&s)),
            fiscal_quarter: str_field(r, "fiscalQuarterEnding").filter(|s| !s.is_empty()),
        });
    }
    if out.is_empty() && skipped > 0 {
        return Err(fail(format!("{skipped} rows, none with a symbol")));
    }
    Ok(out)
}

fn str_field(r: &Value, key: &str) -> Option<String> {
    r.get(key)
        .and_then(Value::as_str)
        .map(|s| s.trim().to_string())
}

fn slot(time: Option<&str>) -> ReportSlot {
    match time {
        Some("time-pre-market") => ReportSlot::BeforeOpen,
        Some("time-after-hours") => ReportSlot::AfterClose,
        _ => ReportSlot::Unspecified,
    }
}

/// "$28,065,267,700" → 28065267700.0; "($0.12)" → -0.12; "N/A" or "" → None.
fn money(raw: &str) -> Option<f64> {
    let s = raw.trim();
    if s.is_empty() || s.eq_ignore_ascii_case("n/a") {
        return None;
    }
    let negative = s.starts_with('(') && s.ends_with(')') || s.starts_with('-');
    let digits: String = s
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let n: f64 = digits.parse().ok()?;
    Some(if negative { -n } else { n })
}

#[cfg(test)]
mod tests {
    use super::*;

    const HAPPY: &str = r#"{"data":{"asOf":"Tue, Sep 8, 2026","rows":[
        {"time":"time-after-hours","symbol":"CASY","name":"Caseys General Stores, Inc.","marketCap":"$28,065,267,700","epsForecast":"$6.60","fiscalQuarterEnding":"Jul/2026"},
        {"time":"time-pre-market","symbol":"ABM","name":"ABM Industries","marketCap":"$3,100,000,000","epsForecast":"($0.12)","fiscalQuarterEnding":"Jul/2026"},
        {"time":"time-not-supplied","symbol":"INNV","name":"InnovAge","marketCap":"N/A","epsForecast":"","fiscalQuarterEnding":""},
        {"time":"time-after-hours","symbol":"","name":"nameless"}
    ]},"message":null,"status":{"rCode":200}}"#;

    #[test]
    fn parses_slots_money_and_skips_symbolless() {
        let rows = parse_rows(HAPPY).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].slot, ReportSlot::AfterClose);
        assert_eq!(rows[0].market_cap_usd, Some(28_065_267_700.0));
        assert_eq!(rows[0].eps_forecast, Some(6.60));
        assert_eq!(rows[1].slot, ReportSlot::BeforeOpen);
        assert_eq!(rows[1].eps_forecast, Some(-0.12));
        assert_eq!(rows[2].slot, ReportSlot::Unspecified);
        assert_eq!(rows[2].market_cap_usd, None);
        assert_eq!(rows[2].eps_forecast, None);
        assert_eq!(rows[2].fiscal_quarter, None);
    }

    #[test]
    fn empty_day_is_empty_and_malformed_is_error() {
        assert!(
            parse_rows(r#"{"data":{"rows":null},"status":{"rCode":200}}"#)
                .unwrap()
                .is_empty()
        );
        assert!(parse_rows(r#"{"status":{"rCode":400}}"#).is_err());
        assert!(parse_rows("not json").is_err());
        assert!(parse_rows(r#"{"data":{"rows":[{"name":"x"}]}}"#).is_err());
    }

    #[tokio::test]
    #[ignore = "live network: hits Nasdaq's calendar endpoint"]
    async fn live_weekday_has_rows() {
        let rows = NasdaqCalendar::new()
            .unwrap()
            .on(NaiveDate::from_ymd_opt(2026, 9, 8).unwrap())
            .await
            .unwrap();
        assert!(!rows.is_empty());
    }
}
