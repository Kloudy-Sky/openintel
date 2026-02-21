use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Market,
    Newsletter,
    Social,
    /// Broader trading strategy and domain analysis (e.g., trading setups, strategy reviews).
    /// Contrast with `Trade`, which represents individual trade records.
    Trading,
    Opportunity,
    Competitor,
    General,
    // New categories for trading intelligence
    Signal,
    Catalyst,
    Weather,
    Macro,
    Sentiment,
    Research,
    Position,
    /// Individual trade records and entries (e.g., a specific buy/sell, a trade loss).
    /// Contrast with `Trading`, which covers broader trading strategy/domain analysis.
    Trade,
    Economic,
    Crypto,
    Portfolio,
    /// System-level intel tracking — heartbeat state observations worth logging.
    /// Intentionally included as a category for pragmatic observability, not a domain concept.
    Heartbeat,
}

impl fmt::Display for Category {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Category::Market => write!(f, "market"),
            Category::Newsletter => write!(f, "newsletter"),
            Category::Social => write!(f, "social"),
            Category::Trading => write!(f, "trading"),
            Category::Opportunity => write!(f, "opportunity"),
            Category::Competitor => write!(f, "competitor"),
            Category::General => write!(f, "general"),
            Category::Signal => write!(f, "signal"),
            Category::Catalyst => write!(f, "catalyst"),
            Category::Weather => write!(f, "weather"),
            Category::Macro => write!(f, "macro"),
            Category::Sentiment => write!(f, "sentiment"),
            Category::Research => write!(f, "research"),
            Category::Position => write!(f, "position"),
            Category::Trade => write!(f, "trade"),
            Category::Economic => write!(f, "economic"),
            Category::Crypto => write!(f, "crypto"),
            Category::Portfolio => write!(f, "portfolio"),
            Category::Heartbeat => write!(f, "heartbeat"),
        }
    }
}

impl FromStr for Category {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let normalized = s.to_lowercase().replace('-', "_");

        match normalized.as_str() {
            // Original categories
            "market" => Ok(Category::Market),
            "newsletter" => Ok(Category::Newsletter),
            "social" => Ok(Category::Social),
            "trading" => Ok(Category::Trading),
            "opportunity" => Ok(Category::Opportunity),
            "competitor" => Ok(Category::Competitor),
            "general" => Ok(Category::General),

            // New categories (direct)
            "signal" => Ok(Category::Signal),
            "catalyst" => Ok(Category::Catalyst),
            "weather" => Ok(Category::Weather),
            "macro" => Ok(Category::Macro),
            "sentiment" => Ok(Category::Sentiment),
            "research" => Ok(Category::Research),
            "position" => Ok(Category::Position),
            "trade" => Ok(Category::Trade),
            "economic" => Ok(Category::Economic),
            "crypto" => Ok(Category::Crypto),
            "portfolio" => Ok(Category::Portfolio),
            "heartbeat" => Ok(Category::Heartbeat),

            // Flexible aliases
            "market_signal" => Ok(Category::Signal),
            "market_analysis" => Ok(Category::Market),
            "economic_data" => Ok(Category::Economic),
            "crypto_signal" => Ok(Category::Crypto),
            "portfolio_status" => Ok(Category::Portfolio),
            "portfolio_update" => Ok(Category::Portfolio),
            "heartbeat_summary" => Ok(Category::Heartbeat),
            "weather_signal" => Ok(Category::Weather),
            "trade_loss" => Ok(Category::Trade),

            _ => Err(format!("Unknown category: {s}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_all_direct_variants() {
        let cases = [
            ("market", Category::Market),
            ("newsletter", Category::Newsletter),
            ("social", Category::Social),
            ("trading", Category::Trading),
            ("opportunity", Category::Opportunity),
            ("competitor", Category::Competitor),
            ("general", Category::General),
            ("signal", Category::Signal),
            ("catalyst", Category::Catalyst),
            ("weather", Category::Weather),
            ("macro", Category::Macro),
            ("sentiment", Category::Sentiment),
            ("research", Category::Research),
            ("position", Category::Position),
            ("trade", Category::Trade),
            ("economic", Category::Economic),
            ("crypto", Category::Crypto),
            ("portfolio", Category::Portfolio),
            ("heartbeat", Category::Heartbeat),
        ];
        for (input, expected) in cases {
            assert_eq!(
                Category::from_str(input).unwrap(),
                expected,
                "failed for {input}"
            );
        }
    }

    #[test]
    fn test_display_roundtrip_all_variants() {
        let variants = [
            Category::Market,
            Category::Newsletter,
            Category::Social,
            Category::Trading,
            Category::Opportunity,
            Category::Competitor,
            Category::General,
            Category::Signal,
            Category::Catalyst,
            Category::Weather,
            Category::Macro,
            Category::Sentiment,
            Category::Research,
            Category::Position,
            Category::Trade,
            Category::Economic,
            Category::Crypto,
            Category::Portfolio,
            Category::Heartbeat,
        ];
        for variant in variants {
            let s = variant.to_string();
            let parsed = Category::from_str(&s).unwrap();
            assert_eq!(parsed, variant, "roundtrip failed for {variant:?}");
        }
    }

    #[test]
    fn test_aliases() {
        assert_eq!(
            Category::from_str("market_signal").unwrap(),
            Category::Signal
        );
        assert_eq!(
            Category::from_str("market_analysis").unwrap(),
            Category::Market
        );
        assert_eq!(
            Category::from_str("economic_data").unwrap(),
            Category::Economic
        );
        assert_eq!(
            Category::from_str("crypto_signal").unwrap(),
            Category::Crypto
        );
        assert_eq!(
            Category::from_str("portfolio_status").unwrap(),
            Category::Portfolio
        );
        assert_eq!(
            Category::from_str("portfolio_update").unwrap(),
            Category::Portfolio
        );
        assert_eq!(
            Category::from_str("heartbeat_summary").unwrap(),
            Category::Heartbeat
        );
        assert_eq!(
            Category::from_str("weather_signal").unwrap(),
            Category::Weather
        );
    }

    #[test]
    fn test_trade_loss_maps_to_trade_not_trading() {
        let result = Category::from_str("trade_loss").unwrap();
        assert_eq!(
            result,
            Category::Trade,
            "trade_loss should map to Trade, not Trading"
        );
        assert_ne!(result, Category::Trading);
    }

    #[test]
    fn test_case_insensitive_and_hyphen_normalization() {
        assert_eq!(Category::from_str("MARKET").unwrap(), Category::Market);
        assert_eq!(Category::from_str("Trade-Loss").unwrap(), Category::Trade);
        assert_eq!(
            Category::from_str("CRYPTO_SIGNAL").unwrap(),
            Category::Crypto
        );
    }

    #[test]
    fn test_unknown_category_error() {
        assert!(Category::from_str("nonexistent").is_err());
        assert!(Category::from_str("").is_err());
    }
}
