use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TradeDirection {
    Long,
    Short,
    Yes,
    No,
}

impl fmt::Display for TradeDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TradeDirection::Long => write!(f, "long"),
            TradeDirection::Short => write!(f, "short"),
            TradeDirection::Yes => write!(f, "yes"),
            TradeDirection::No => write!(f, "no"),
        }
    }
}

impl FromStr for TradeDirection {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "long" => Ok(TradeDirection::Long),
            "short" => Ok(TradeDirection::Short),
            "yes" => Ok(TradeDirection::Yes),
            "no" => Ok(TradeDirection::No),
            _ => Err(format!("Unknown trade direction: {s}")),
        }
    }
}
