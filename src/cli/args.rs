use clap::{Parser, Subcommand, ValueEnum};

use crate::config::settings::{AppConfig, OutputFormat};

#[derive(Parser, Debug)]
#[command(
    name = "openintel",
    version,
    about = "Fuse social sentiment with market action into a speculation report"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Analyze a ticker across social + market sources
    Analyze(AnalyzeArgs),

    /// Run as an MCP server over stdio (for AI agents).
    Mcp,

    /// Guided setup + live check for a data source (env-only; never stores credentials)
    Setup(SetupArgs),
}

#[derive(clap::Args, Debug)]
pub struct AnalyzeArgs {
    /// Ticker symbol, e.g. AAPL
    pub ticker: String,

    #[arg(long)]
    pub enable_reddit: bool,
    #[arg(long)]
    pub enable_x: bool,
    #[arg(long)]
    pub enable_bluesky: bool,

    /// Skip the market snapshot (social-only report)
    #[arg(long)]
    pub no_market: bool,

    /// Posts to fetch per source
    #[arg(long, default_value_t = 50)]
    pub limit: usize,

    #[arg(long, value_enum, default_value_t = FormatArg::Table)]
    pub format: FormatArg,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormatArg {
    Table,
    Json,
}

#[derive(clap::Args, Debug)]
pub struct SetupArgs {
    /// Which source to set up
    #[arg(value_enum)]
    pub source: SetupSource,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupSource {
    Reddit,
}

pub fn to_app_config(args: &AnalyzeArgs) -> AppConfig {
    let format = match args.format {
        FormatArg::Table => OutputFormat::Table,
        FormatArg::Json => OutputFormat::Json,
    };
    AppConfig::new(
        args.ticker.clone(),
        args.enable_reddit,
        args.enable_x,
        args.enable_bluesky,
        args.no_market,
        args.limit,
        format,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parses_analyze_with_json_format() {
        let cli =
            Cli::try_parse_from(["openintel", "analyze", "AAPL", "--format", "json"]).unwrap();
        let Command::Analyze(args) = cli.command else {
            unreachable!()
        };
        assert_eq!(args.ticker, "AAPL");
        assert_eq!(args.format, FormatArg::Json);
        assert_eq!(args.limit, 50);
    }

    #[test]
    fn maps_no_flags_to_all_sources() {
        let cli = Cli::try_parse_from(["openintel", "analyze", "MSFT"]).unwrap();
        let Command::Analyze(args) = cli.command else {
            unreachable!()
        };
        let cfg = to_app_config(&args);
        assert_eq!(cfg.enabled_sources.len(), 3);
        assert!(cfg.market_enabled);
        assert_eq!(cfg.format, crate::config::settings::OutputFormat::Table);
    }

    #[test]
    fn parses_setup_reddit() {
        let cli = Cli::try_parse_from(["openintel", "setup", "reddit"]).unwrap();
        let Command::Setup(args) = cli.command else {
            panic!("expected setup command");
        };
        assert_eq!(args.source, SetupSource::Reddit);
    }

    #[test]
    fn rejects_unknown_setup_source() {
        assert!(Cli::try_parse_from(["openintel", "setup", "bogus"]).is_err());
    }
}
