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

    /// Deterministic risk math for one trade idea: ATR stop, budget-capped size, R targets
    Risk(RiskArgs),

    /// Scan the day's biggest losers for gated dip setups (grades conformance, never advises)
    Dip(DipArgs),
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

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectionArg {
    Long,
    Short,
}

#[derive(clap::Args, Debug)]
pub struct RiskArgs {
    /// Ticker symbol, e.g. NVDA
    pub ticker: String,

    /// Per-trade risk budget in USD — the most a stop-out may lose
    #[arg(long)]
    pub budget: f64,

    #[arg(long, value_enum, default_value_t = DirectionArg::Long)]
    pub direction: DirectionArg,

    /// Stop distance in ATR multiples (0.5-5)
    #[arg(long = "stop-mult", default_value_t = 2.0)]
    pub stop_mult: f64,

    /// Entry price override (default: last close)
    #[arg(long)]
    pub entry: Option<f64>,

    #[arg(long, value_enum, default_value_t = FormatArg::Table)]
    pub format: FormatArg,
}

#[derive(clap::Args, Debug)]
pub struct DipArgs {
    /// Evaluate one symbol instead of scanning the losers universe
    pub ticker: Option<String>,

    /// Grade the scan journal against forward returns instead of scanning
    #[arg(long, conflicts_with = "ticker")]
    pub review: bool,

    /// Losers to pull from the screener (1-100)
    #[arg(long, default_value_t = 100)]
    pub count: usize,

    /// Floor+band survivors to deep-analyze (1-25)
    #[arg(long, default_value_t = 10)]
    pub deep: usize,

    /// Worst day-change eligible, in percent
    #[arg(long = "band-min", default_value_t = -15.0, allow_negative_numbers = true)]
    pub band_min: f64,

    /// Mildest day-change eligible, in percent
    #[arg(long = "band-max", default_value_t = -4.0, allow_negative_numbers = true)]
    pub band_max: f64,

    /// Account equity in USD — enables the risk + margin sizing section
    #[arg(long)]
    pub equity: Option<f64>,

    /// Buying-power multiple (overnight Reg-T caps at 2)
    #[arg(long, default_value_t = 2.0)]
    pub leverage: f64,

    /// Unlock 4x intraday buying power (not holdable overnight)
    #[arg(long)]
    pub intraday_bp: bool,

    /// Maintenance requirement fraction
    #[arg(long, default_value_t = 0.25)]
    pub maintenance: f64,

    /// Fraction of equity risked to the stop per position
    #[arg(long = "risk-pct", default_value_t = 0.01)]
    pub risk_pct: f64,

    /// Minimum composite score for the score gate
    #[arg(long = "score-min", default_value_t = 65.0)]
    pub score_min: f64,

    /// Quality floor: minimum share price
    #[arg(long = "min-price", default_value_t = 5.0)]
    pub min_price: f64,

    /// Quality floor: minimum market cap in USD
    #[arg(long = "min-cap", default_value_t = 500_000_000)]
    pub min_cap: u64,

    /// Quality floor: minimum 3-month average daily volume in shares
    #[arg(long = "min-volume", default_value_t = 1_000_000)]
    pub min_volume: u64,

    /// Quality floor: minimum days since listing
    #[arg(long = "min-listed-days", default_value_t = 180)]
    pub min_listed_days: i64,

    /// Skip the scan journal (~/.openintel/dip_journal.jsonl)
    #[arg(long)]
    pub no_journal: bool,

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

    #[test]
    fn parses_risk_args() {
        let cli = Cli::try_parse_from([
            "openintel",
            "risk",
            "NVDA",
            "--budget",
            "200",
            "--direction",
            "short",
            "--stop-mult",
            "1.5",
        ])
        .unwrap();
        let Command::Risk(args) = cli.command else {
            panic!("expected risk command");
        };
        assert_eq!(args.ticker, "NVDA");
        assert_eq!(args.budget, 200.0);
        assert_eq!(args.direction, DirectionArg::Short);
        assert_eq!(args.stop_mult, 1.5);
        assert!(args.entry.is_none());
    }

    #[test]
    fn risk_requires_budget() {
        assert!(Cli::try_parse_from(["openintel", "risk", "NVDA"]).is_err());
    }
}
