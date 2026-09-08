use serde::Serialize;

use crate::domain::error::DomainError;
use crate::domain::values::asset_class::AssetClass;

/// Bare crypto bases that expand to Yahoo's `-USD` form. Each was checked
/// against listed US equity tickers when added; a base that collides with a
/// listed equity must not be added here, because the bare form would then
/// stop naming the stock.
const CRYPTO_SHORTHAND: &[&str] = &[
    "BTC", "ETH", "SOL", "XRP", "DOGE", "ADA", "AVAX", "DOT", "LTC", "BCH", "UNI", "SHIB", "XLM",
    "NEAR",
];

/// A validated market symbol in Yahoo's form, which the keyless adapters
/// share: `AAPL` / `BRK.B` (equity), `BTC-USD` (crypto), `EURUSD=X` (forex),
/// `ES=F` (future). The form carries the asset class.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Ticker(String);

fn upper_alpha(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_uppercase())
}

fn upper_alnum(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

impl Ticker {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(DomainError::InvalidTicker("empty".into()));
        }
        if !trimmed.is_ascii() {
            return Err(DomainError::InvalidTicker(raw.to_string()));
        }
        let symbol = trimmed.to_ascii_uppercase();
        let reject = || DomainError::InvalidTicker(raw.to_string());

        if let Some(pair) = symbol.strip_suffix("=X") {
            return (pair.len() == 6 && upper_alpha(pair))
                .then(|| Ticker(symbol.clone()))
                .ok_or_else(reject);
        }
        if let Some(root) = symbol.strip_suffix("=F") {
            return ((1..=4).contains(&root.len()) && upper_alnum(root))
                .then(|| Ticker(symbol.clone()))
                .ok_or_else(reject);
        }
        if let Some((base, quote)) = symbol.split_once('-') {
            let ok = (2..=10).contains(&base.len())
                && upper_alnum(base)
                && (3..=5).contains(&quote.len())
                && upper_alpha(quote);
            return ok.then(|| Ticker(symbol.clone())).ok_or_else(reject);
        }
        if CRYPTO_SHORTHAND.contains(&symbol.as_str()) {
            return Ok(Ticker(format!("{symbol}-USD")));
        }

        let (base, class) = match symbol.split_once('.') {
            Some((b, c)) => (b, Some(c)),
            None => (symbol.as_str(), None),
        };
        let base_ok = (1..=5).contains(&base.len()) && upper_alpha(base);
        let class_ok = class.is_none_or(|c| c.len() == 1 && upper_alpha(c));
        if base_ok && class_ok {
            Ok(Ticker(symbol))
        } else {
            Err(reject())
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The part people write in a cashtag: `BTC` for `BTC-USD`, `EURUSD` for
    /// `EURUSD=X`, `ES` for `ES=F`, the symbol itself for an equity.
    pub fn base(&self) -> &str {
        let s = self.0.as_str();
        s.split_once('-')
            .map(|(b, _)| b)
            .or_else(|| s.strip_suffix("=X"))
            .or_else(|| s.strip_suffix("=F"))
            .unwrap_or(s)
    }

    pub fn class(&self) -> AssetClass {
        if self.0.ends_with("=X") {
            AssetClass::Forex
        } else if self.0.ends_with("=F") {
            AssetClass::Future
        } else if self.0.contains('-') {
            AssetClass::Crypto
        } else {
            AssetClass::Equity
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_equities() {
        assert_eq!(Ticker::parse("aapl").unwrap().as_str(), "AAPL");
        assert_eq!(Ticker::parse("BRK.B").unwrap().as_str(), "BRK.B");
        assert_eq!(Ticker::parse("AAPL").unwrap().class(), AssetClass::Equity);
    }

    #[test]
    fn accepts_yahoo_forms_for_other_classes() {
        let btc = Ticker::parse("btc-usd").unwrap();
        assert_eq!(btc.as_str(), "BTC-USD");
        assert_eq!(btc.class(), AssetClass::Crypto);
        assert_eq!(
            Ticker::parse("1INCH-USD").unwrap().class(),
            AssetClass::Crypto
        );
        let fx = Ticker::parse("eurusd=x").unwrap();
        assert_eq!(fx.as_str(), "EURUSD=X");
        assert_eq!(fx.class(), AssetClass::Forex);
        let fut = Ticker::parse("ES=F").unwrap();
        assert_eq!(fut.class(), AssetClass::Future);
    }

    #[test]
    fn bare_crypto_shorthand_expands_to_usd() {
        assert_eq!(Ticker::parse("btc").unwrap().as_str(), "BTC-USD");
        assert_eq!(Ticker::parse("btc").unwrap().base(), "BTC");
        assert_eq!(Ticker::parse("EURUSD=X").unwrap().base(), "EURUSD");
        assert_eq!(Ticker::parse("ES=F").unwrap().base(), "ES");
        assert_eq!(Ticker::parse("BRK.B").unwrap().base(), "BRK.B");
        assert_eq!(Ticker::parse("ETH").unwrap().class(), AssetClass::Crypto);
        // a five-letter equity that is not in the shorthand list stays an equity
        assert_eq!(Ticker::parse("GOOGL").unwrap().class(), AssetClass::Equity);
    }

    #[test]
    fn rejects_invalid_symbols() {
        for bad in [
            "",
            "   ",
            "TOOLONG",
            "A1",
            "AB.CD",
            "AAPL.",
            "$AAPL",
            "ß",
            "ﬁ",
            "BTC-",
            "-USD",
            "A-B",
            "BTC-US1",
            "EURUS=X",
            "EURUSDX=X",
            "=F",
            "TOOLONG=F",
        ] {
            assert!(
                Ticker::parse(bad).is_err(),
                "expected {bad:?} to be rejected"
            );
        }
    }
}
