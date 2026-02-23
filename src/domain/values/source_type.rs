use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceType {
    /// Intelligence from external sources (newsletters, market data, social media)
    External,
    /// Internal operational entries (agent logs, heartbeat notes)
    Internal,
}

impl Default for SourceType {
    fn default() -> Self {
        Self::External
    }
}

impl fmt::Display for SourceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::External => write!(f, "external"),
            Self::Internal => write!(f, "internal"),
        }
    }
}

impl FromStr for SourceType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "external" | "ext" => Ok(Self::External),
            "internal" | "int" => Ok(Self::Internal),
            _ => Err(format!("Invalid source type: '{}'. Use 'external' or 'internal'", s)),
        }
    }
}
