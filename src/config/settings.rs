use crate::domain::engine::config::EngineConfig;
use crate::domain::values::source_kind::SourceKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Table,
    Json,
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub ticker: String,
    pub enabled_sources: Vec<SourceKind>,
    pub market_enabled: bool,
    pub limit: usize,
    pub format: OutputFormat,
    pub engine: EngineConfig,
}

impl AppConfig {
    pub fn new(
        ticker: String,
        reddit: bool,
        bluesky: bool,
        no_market: bool,
        limit: usize,
        format: OutputFormat,
    ) -> Self {
        let mut enabled_sources = Vec::new();
        if reddit {
            enabled_sources.push(SourceKind::Reddit);
        }
        if bluesky {
            enabled_sources.push(SourceKind::Bluesky);
        }
        if enabled_sources.is_empty() {
            enabled_sources = SourceKind::ALL.to_vec();
        }

        AppConfig {
            ticker,
            enabled_sources,
            market_enabled: !no_market,
            limit,
            format,
            engine: EngineConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_flags_enables_all_sources_and_market() {
        let c = AppConfig::new("AAPL".into(), false, false, false, 50, OutputFormat::Table);
        assert_eq!(
            c.enabled_sources,
            vec![SourceKind::Reddit, SourceKind::Bluesky]
        );
        assert!(c.market_enabled);
    }

    #[test]
    fn single_flag_narrows_sources() {
        let c = AppConfig::new("AAPL".into(), true, false, true, 50, OutputFormat::Json);
        assert_eq!(c.enabled_sources, vec![SourceKind::Reddit]);
        assert!(!c.market_enabled);
    }
}
