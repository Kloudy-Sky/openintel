use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Polarity(f64);

impl Polarity {
    pub fn new(value: f64) -> Self {
        if value.is_nan() {
            Polarity(0.0)
        } else {
            Polarity(value.clamp(-1.0, 1.0))
        }
    }

    pub fn value(self) -> f64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_out_of_range() {
        assert_eq!(Polarity::new(5.0).value(), 1.0);
        assert_eq!(Polarity::new(-5.0).value(), -1.0);
        assert_eq!(Polarity::new(0.3).value(), 0.3);
    }

    #[test]
    fn nan_becomes_zero() {
        assert_eq!(Polarity::new(f64::NAN).value(), 0.0);
    }
}
