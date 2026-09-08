use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// What a watch poll noticed. Each is a dated fact with its evidence; none is
/// a recommendation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// A catalyst-form SEC filing appeared for a watched name.
    Filing,
    /// A company-referencing catalyst headline appeared for a watched name.
    CatalystHeadline,
    /// Mention velocity on a platform crossed the chatter honesty gates.
    ChatterVelocity,
    /// Price moved a further whole multiple of ATR from the prior close.
    PriceMove,
    /// The equity market's session state changed.
    MarketState,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// When the poll that saw it ran: the latency ceiling of a keyless watch.
    pub polled_at: DateTime<Utc>,
    pub kind: EventKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ticker: Option<String>,
    pub summary: String,
    pub evidence: Vec<String>,
    pub source: String,
}
