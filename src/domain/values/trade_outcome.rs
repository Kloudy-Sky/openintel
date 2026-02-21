use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TradeOutcome {
    Win,
    Loss,
    Scratch,
}

impl fmt::Display for TradeOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TradeOutcome::Win => write!(f, "win"),
            TradeOutcome::Loss => write!(f, "loss"),
            TradeOutcome::Scratch => write!(f, "scratch"),
        }
    }
}

impl FromStr for TradeOutcome {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "win" => Ok(TradeOutcome::Win),
            "loss" => Ok(TradeOutcome::Loss),
            "scratch" => Ok(TradeOutcome::Scratch),
            _ => Err(format!("Unknown trade outcome: {s}")),
        }
    }
}
