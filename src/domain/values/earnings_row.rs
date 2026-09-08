use serde::Serialize;

/// When in the day a company reports, as the calendar provider states it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportSlot {
    BeforeOpen,
    AfterClose,
    Unspecified,
}

/// One scheduled earnings report. Market cap and forecast are provider
/// fields, absent when the provider says N/A — never defaulted to zero.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EarningsRow {
    pub symbol: String,
    pub company: String,
    pub slot: ReportSlot,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub market_cap_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eps_forecast: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fiscal_quarter: Option<String>,
}
