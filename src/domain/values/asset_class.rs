use serde::{Deserialize, Serialize};

/// What kind of instrument a symbol names. Inferred from the symbol's form
/// (see `Ticker::class`), so it travels with the symbol and nothing
/// downstream guesses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetClass {
    Equity,
    Crypto,
    Forex,
    Future,
}

impl AssetClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Equity => "equity",
            Self::Crypto => "crypto",
            Self::Forex => "forex",
            Self::Future => "future",
        }
    }
}
