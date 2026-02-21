use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Confidence(f64);

impl Confidence {
    pub fn new(value: f64) -> Result<Self, String> {
        if !(0.0..=1.0).contains(&value) {
            return Err(format!(
                "Confidence must be between 0.0 and 1.0, got {value}"
            ));
        }
        Ok(Confidence(value))
    }

    pub fn value(&self) -> f64 {
        self.0
    }
}

impl fmt::Display for Confidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.2}", self.0)
    }
}

impl Default for Confidence {
    fn default() -> Self {
        Confidence(0.5)
    }
}
