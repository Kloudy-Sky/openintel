use serde::Serialize;

use crate::domain::ports::market_data_source::MarketDataSource;
use crate::domain::values::source_kind::SourceKind;

// NOTE: Serialize-ONLY (no JsonSchema). This is deliberate — it makes `list_sources`
// returning `Json<SourcesOutput>` the spike that proves `Json<T>` works with a
// Serialize-only payload. The report-bearing outputs (Tasks 3-5) nest the
// Serialize-only `SpeculationReport`, so they cannot derive JsonSchema. If this
// compiles, `Json<T>` needs only Serialize and those tools are fine as written.
#[derive(Debug, Serialize)]
pub struct SourcesOutput {
    pub social: Vec<String>,
    pub market: Vec<String>,
}

/// Derived from `SourceKind::ALL` (one source of truth) + the market adapter's name.
pub fn run_list_sources() -> SourcesOutput {
    SourcesOutput {
        social: SourceKind::ALL
            .iter()
            .map(|s| s.as_str().to_string())
            .collect(),
        market: vec![crate::adapters::market::mock_market::MockMarketSource
            .name()
            .to_string()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_sources_reports_all_adapters() {
        let out = run_list_sources();
        assert_eq!(out.social, vec!["reddit", "x", "bluesky"]);
        assert_eq!(out.market, vec!["mock-market"]);
    }
}
