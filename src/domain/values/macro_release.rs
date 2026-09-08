use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// One scheduled macro data release or policy decision (CPI, jobs, FOMC, GDP, PCE).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MacroRelease {
    pub date: NaiveDate,
    /// Wall-clock time in New York, "HH:MM".
    pub time_et: String,
    pub release: String,
    pub agency: String,
}
