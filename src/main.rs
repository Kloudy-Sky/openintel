use std::process::ExitCode;

use clap::Parser;

use openintel::adapters::market::yahoo::YahooMarketSource;
use openintel::adapters::sources::mock_bluesky::MockBlueskySource;
use openintel::adapters::sources::mock_x::MockXSource;
use openintel::adapters::sources::reddit::RedditSource;
use openintel::cli::args::{to_app_config, Cli, Command};
use openintel::cli::run::analyze;
use openintel::config::secrets::Credentials;
use openintel::domain::ports::social_data_source::SocialDataSource;

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    // Reddit client credentials (if set) enable the real Reddit source; other sources need none.
    let credentials = Credentials::from_env();

    match cli.command {
        Command::Analyze(args) => {
            let config = to_app_config(&args);

            let mut social: Vec<Box<dyn SocialDataSource>> = Vec::new();
            if let (Some(id), Some(secret)) = (
                credentials.reddit_client_id,
                credentials.reddit_client_secret,
            ) {
                match RedditSource::new(id, secret) {
                    Ok(src) => social.push(Box::new(src)),
                    Err(e) => eprintln!("warning: reddit disabled: {e}"),
                }
            }
            social.push(Box::new(MockXSource));
            social.push(Box::new(MockBlueskySource));

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
    }
}
