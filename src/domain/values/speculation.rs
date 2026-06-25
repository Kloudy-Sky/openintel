use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(transparent)]
pub struct SpeculationIndex(f64);

impl SpeculationIndex {
    pub fn new(value: f64) -> Self {
        SpeculationIndex(value.clamp(0.0, 1.0))
    }

    pub fn value(self) -> f64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    Low,
    Medium,
    High,
}

impl Confidence {
    /// `n < low` -> Low, `low <= n < high` -> Medium, `n >= high` -> High.
    pub fn from_sample(n: usize, low: usize, high: usize) -> Self {
        if n < low {
            Confidence::Low
        } else if n < high {
            Confidence::Medium
        } else {
            Confidence::High
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Alignment {
    ConfirmingBullish,
    ConfirmingBearish,
    Diverging,
    Quiet,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speculation_index_clamps() {
        assert_eq!(SpeculationIndex::new(1.5).value(), 1.0);
        assert_eq!(SpeculationIndex::new(-0.2).value(), 0.0);
        assert_eq!(SpeculationIndex::new(0.5).value(), 0.5);
    }

    #[test]
    fn confidence_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&Confidence::Low).unwrap(), "\"low\"");
        assert_eq!(
            serde_json::to_string(&Confidence::High).unwrap(),
            "\"high\""
        );
    }

    #[test]
    fn alignment_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&Alignment::ConfirmingBullish).unwrap(),
            "\"confirming_bullish\""
        );
        assert_eq!(
            serde_json::to_string(&Alignment::Quiet).unwrap(),
            "\"quiet\""
        );
    }

    #[test]
    fn confidence_buckets() {
        assert_eq!(Confidence::from_sample(5, 10, 50), Confidence::Low);
        assert_eq!(Confidence::from_sample(10, 10, 50), Confidence::Medium);
        assert_eq!(Confidence::from_sample(49, 10, 50), Confidence::Medium);
        assert_eq!(Confidence::from_sample(50, 10, 50), Confidence::High);
    }
}
