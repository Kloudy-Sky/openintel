use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::domain::entities::ticker::Ticker;
use crate::domain::values::polarity::Polarity;
use crate::domain::values::source_kind::SourceKind;
use crate::domain::values::speculation::{Alignment, Confidence, SpeculationIndex};

#[derive(Debug, Clone, Serialize)]
pub struct SocialSummary {
    pub total_mentions: usize,
    pub mentions_by_source: BTreeMap<SourceKind, usize>,
    pub net_sentiment: Polarity,
    pub bullish: usize,
    pub bearish: usize,
    pub neutral: usize,
    pub bull_bear_ratio: Option<f64>,
    pub speculation_index: SpeculationIndex,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarketSummary {
    pub last_price: f64,
    pub pct_change: f64,
    pub rvol: Option<f64>,
    pub realized_vol: Option<f64>,
    pub put_call_ratio: Option<f64>,
    pub iv_rank: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FusionSignals {
    pub alignment: Alignment,
    pub crowding: f64,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpeculationReport {
    pub ticker: Ticker,
    pub generated_at: DateTime<Utc>,
    pub social: SocialSummary,
    pub market: Option<MarketSummary>,
    pub fusion: FusionSignals,
    pub social_confidence: Confidence,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn serializes_with_enum_source_keys() {
        let mut by_source = BTreeMap::new();
        by_source.insert(SourceKind::Reddit, 2);
        let report = SpeculationReport {
            ticker: Ticker::parse("AAPL").unwrap(),
            generated_at: Utc.with_ymd_and_hms(2026, 6, 24, 0, 0, 0).unwrap(),
            social: SocialSummary {
                total_mentions: 2,
                mentions_by_source: by_source,
                net_sentiment: Polarity::new(0.4),
                bullish: 2,
                bearish: 0,
                neutral: 0,
                bull_bear_ratio: None,
                speculation_index: SpeculationIndex::new(0.5),
            },
            market: None,
            fusion: FusionSignals {
                alignment: Alignment::Quiet,
                crowding: 0.25,
                notes: vec![],
            },
            social_confidence: Confidence::Low,
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"reddit\":2"));
        assert!(json.contains("\"speculation_index\":0.5"));
        assert!(json.contains("\"alignment\":\"quiet\""));
    }
}
