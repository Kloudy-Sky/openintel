pub mod analyze;
pub mod request;

pub use analyze::analyze;
pub use request::AnalysisRequest;

/// Appended to every analysis-bearing output (CLI renders it; MCP returns it in a
/// `disclaimer` field). Single source of truth — do not duplicate this string.
pub const DISCLAIMER: &str = "Not financial advice. OpenIntel is a research/screening tool; \
markets are risky and social data is easily manipulated. Do your own diligence.";
