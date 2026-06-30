use std::process::ExitCode;

use clap::Parser;

use openintel::cli::args::{to_app_config, Cli, Command};
use openintel::cli::run::analyze;
use openintel::config::secrets::Credentials;

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    // Loaded for future real adapters; mock adapters ignore credentials.
    let _credentials = Credentials::from_env();

    match cli.command {
        Command::Analyze(args) => {
            let config = to_app_config(&args);
            match analyze(&config).await {
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
    }
}
