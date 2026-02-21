use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Market,
    Newsletter,
    Social,
    Trading,
    Opportunity,
    Competitor,
    General,
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
        }
    }
}

impl FromStr for Category {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "market" => Ok(Category::Market),
            "newsletter" => Ok(Category::Newsletter),
            "social" => Ok(Category::Social),
            "trading" => Ok(Category::Trading),
            "opportunity" => Ok(Category::Opportunity),
            "competitor" => Ok(Category::Competitor),
            "general" => Ok(Category::General),
            _ => Err(format!("Unknown category: {s}")),
        }
    }
}
