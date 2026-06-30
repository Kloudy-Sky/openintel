use serde::Serialize;

use crate::domain::error::DomainError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Ticker(String);

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

        let (base, class) = match symbol.split_once('.') {
            Some((b, c)) => (b, Some(c)),
            None => (symbol.as_str(), None),
        };

        let base_ok = (1..=5).contains(&base.len()) && base.chars().all(|c| c.is_ascii_uppercase());
        let class_ok = match class {
            None => true,
            Some(c) => c.len() == 1 && c.chars().all(|c| c.is_ascii_uppercase()),
        };

        if base_ok && class_ok {
            Ok(Ticker(symbol))
        } else {
            Err(DomainError::InvalidTicker(raw.to_string()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_symbols() {
        assert_eq!(Ticker::parse("aapl").unwrap().as_str(), "AAPL");
        assert_eq!(Ticker::parse("BRK.B").unwrap().as_str(), "BRK.B");
    }

    #[test]
    fn rejects_invalid_symbols() {
        for bad in [
            "", "   ", "TOOLONG", "A1", "AB.CD", "AAPL.", "$AAPL", "ß", "ﬁ",
        ] {
            assert!(
                Ticker::parse(bad).is_err(),
                "expected {bad:?} to be rejected"
            );
        }
    }
}
