use std::process::ExitCode;

use clap::Parser;

use openintel::adapters::market::yahoo::YahooMarketSource;
use openintel::cli::args::{to_app_config, Cli, Command};
use openintel::cli::run::analyze;
use openintel::config::secrets::Credentials;
use openintel::config::store::KeychainStore;

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    // Credentials resolve env-first, then the OS keychain (written by `openintel setup`).
    let store = KeychainStore::new();
    let credentials = Credentials::load(&store);

    match cli.command {
        Command::Analyze(args) => {
            let config = to_app_config(&args);

            let social = openintel::adapters::sources::build_social_sources(&credentials);

            let outcome = if config.market_enabled {
                let market = match YahooMarketSource::new() {
                    Ok(m) => m,
                    Err(e) => {
                        eprintln!("error: {e}");
                        return ExitCode::FAILURE;
                    }
                };
                analyze(&config, &social, Some(&market)).await
            } else {
                analyze(&config, &social, None).await
            };
            match outcome {
                Ok((_report, rendered)) => {
                    println!("{rendered}");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Command::Mcp => match openintel::mcp::server::serve().await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("mcp server error: {e}");
                ExitCode::FAILURE
            }
        },
        Command::Setup(args) => {
            openintel::cli::setup::run(args.source, &credentials, &store, args.forget).await
        }
        Command::Pulse(args) => match openintel::cli::pulse::run(&args, &credentials).await {
            Ok(rendered) => {
                println!("{rendered}");
                ExitCode::SUCCESS
            }
            Err(e) if e.to_string().contains("not configured") => {
                println!("{}", openintel::cli::pulse::not_configured_text());
                ExitCode::FAILURE
            }
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
        Command::Risk(args) => match openintel::cli::risk::run(&args).await {
            Ok(rendered) => {
                println!("{rendered}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
    }
}
