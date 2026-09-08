use chrono::NaiveDate;

/// One regulatory filing (form type + filing date), for the catalyst gate.
/// `accession` is the registry's own id (EDGAR's accession number) so two
/// same-form, same-day filings stay distinct; None when a source has none.
#[derive(Debug, Clone, PartialEq)]
pub struct Filing {
    pub form: String,
    pub filed_on: NaiveDate,
    pub accession: Option<String>,
}
