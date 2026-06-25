#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// τ — per-post bull/bear classification threshold.
    pub bull_bear_threshold: f64,
    /// σ — aggregate net-sentiment threshold for alignment.
    pub net_sentiment_threshold: f64,
    /// δ — minimum |pct_change| (percent) to count as a meaningful price move.
    pub price_move_threshold: f64,
    pub crowding_weight_spec: f64,
    pub crowding_weight_rvol: f64,
    pub crowding_weight_iv: f64,
    pub rvol_cap: f64,
    pub min_sample: usize,
    pub confidence_low: usize,
    pub confidence_high: usize,
}

impl Default for EngineConfig {
    fn default() -> Self {
        EngineConfig {
            bull_bear_threshold: 0.2,
            net_sentiment_threshold: 0.05,
            price_move_threshold: 1.0,
            crowding_weight_spec: 0.5,
            crowding_weight_rvol: 0.3,
            crowding_weight_iv: 0.2,
            rvol_cap: 3.0,
            min_sample: 10,
            confidence_low: 10,
            confidence_high: 50,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_spec() {
        let c = EngineConfig::default();
        assert_eq!(c.bull_bear_threshold, 0.2);
        assert_eq!(c.net_sentiment_threshold, 0.05);
        assert_eq!(c.min_sample, 10);
        assert_eq!((c.confidence_low, c.confidence_high), (10, 50));
    }
}
