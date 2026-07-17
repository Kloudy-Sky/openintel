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

    /// Guided setup + live verify for a data source (saves to the OS keychain; env vars override)
    Setup(SetupArgs),

    /// Catalyst posts from specific high-impact X accounts (paid X API — opt-in)
    Pulse(PulseArgs),
}

#[derive(clap::Args, Debug)]
pub struct AnalyzeArgs {
    /// Ticker symbol, e.g. AAPL
    pub ticker: String,

    #[arg(long)]
    pub enable_reddit: bool,
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

    /// Remove this source's saved credentials from the OS keychain
    #[arg(long)]
    pub forget: bool,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupSource {
    Reddit,
    Bluesky,
    X,
}

#[derive(clap::Args, Debug)]
pub struct PulseArgs {
    /// Ticker symbol, e.g. NVDA
    pub ticker: String,

    /// X handles to listen to, comma-separated (no @). Default: the macro list.
    #[arg(long, value_delimiter = ',')]
    pub accounts: Vec<String>,

    /// Extra search terms in the accounts' own language, comma-separated;
    /// phrases allowed (e.g. tesla,robotaxi,General Motors) — cashtags are
    /// rare in influencer posts
    #[arg(long, value_delimiter = ',')]
    pub keywords: Vec<String>,

    /// Lookback window in hours (1-167)
    #[arg(long, default_value_t = 24)]
    pub hours: u32,

    /// Max posts to read — each costs ~$0.005; X bills a minimum of 10 reads per call (1-100)
    #[arg(long, default_value_t = 20)]
    pub limit: usize,

    #[arg(long, value_enum, default_value_t = FormatArg::Table)]
    pub format: FormatArg,
}

pub fn to_app_config(args: &AnalyzeArgs) -> AppConfig {
    let format = match args.format {
        FormatArg::Table => OutputFormat::Table,
        FormatArg::Json => OutputFormat::Json,
    };
    AppConfig::new(
        args.ticker.clone(),
        args.enable_reddit,
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
        assert_eq!(cfg.enabled_sources.len(), 2);
        assert!(cfg.market_enabled);
        assert_eq!(cfg.format, crate::config::settings::OutputFormat::Table);
    }

    #[test]
    fn enable_x_flag_no_longer_exists() {
        assert!(Cli::try_parse_from(["openintel", "analyze", "AAPL", "--enable-x"]).is_err());
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
    fn parses_setup_bluesky() {
        let cli = Cli::try_parse_from(["openintel", "setup", "bluesky"]).unwrap();
        let Command::Setup(args) = cli.command else {
            panic!("expected setup command");
        };
        assert_eq!(args.source, SetupSource::Bluesky);
    }

    #[test]
    fn parses_setup_x() {
        let cli = Cli::try_parse_from(["openintel", "setup", "x"]).unwrap();
        let Command::Setup(args) = cli.command else {
            panic!("expected setup command");
        };
        assert_eq!(args.source, SetupSource::X);
    }

    #[test]
    fn rejects_unknown_setup_source() {
        assert!(Cli::try_parse_from(["openintel", "setup", "bogus"]).is_err());
    }

    #[test]
    fn parses_setup_forget_flag() {
        let cli = Cli::try_parse_from(["openintel", "setup", "reddit", "--forget"]).unwrap();
        let Command::Setup(args) = cli.command else {
            panic!("expected setup command");
        };
        assert!(args.forget);
    }

    #[test]
    fn parses_pulse_with_accounts() {
        let cli = Cli::try_parse_from([
            "openintel",
            "pulse",
            "NVDA",
            "--accounts",
            "jensenhuang,elonmusk",
            "--hours",
            "48",
        ])
        .unwrap();
        let Command::Pulse(args) = cli.command else {
            panic!("expected pulse command");
        };
        assert_eq!(args.ticker, "NVDA");
        assert_eq!(args.accounts, vec!["jensenhuang", "elonmusk"]);
        assert_eq!(args.hours, 48);
        assert_eq!(args.limit, 20);
    }

    #[test]
    fn pulse_defaults_have_empty_accounts() {
        let cli = Cli::try_parse_from(["openintel", "pulse", "GME"]).unwrap();
        let Command::Pulse(args) = cli.command else {
            panic!("expected pulse command");
        };
        assert!(args.accounts.is_empty());
        assert!(args.keywords.is_empty());
        assert_eq!(args.hours, 24);
    }

    #[test]
    fn parses_pulse_with_keywords() {
        let cli = Cli::try_parse_from([
            "openintel",
            "pulse",
            "TSLA",
            "--accounts",
            "elonmusk",
            "--keywords",
            "tesla,robotaxi",
        ])
        .unwrap();
        let Command::Pulse(args) = cli.command else {
            panic!("expected pulse command");
        };
        assert_eq!(args.keywords, vec!["tesla", "robotaxi"]);
    }
}
