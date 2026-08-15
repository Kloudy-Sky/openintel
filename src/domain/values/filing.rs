use chrono::NaiveDate;

/// One regulatory filing (form type + filing date), for the catalyst gate.
#[derive(Debug, Clone, PartialEq)]
pub struct Filing {
    pub form: String,
    pub filed_on: NaiveDate,
}
