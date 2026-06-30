use crate::domain::values::polarity::Polarity;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PostSignal {
    pub polarity: Polarity,
    pub speculative: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holds_polarity_and_flag() {
        let s = PostSignal {
            polarity: Polarity::new(0.5),
            speculative: true,
        };
        assert_eq!(s.polarity.value(), 0.5);
        assert!(s.speculative);
    }
}
