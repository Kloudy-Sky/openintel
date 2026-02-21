use clap::Parser;
use openintel::cli::commands::{Cli, Commands};
use openintel::domain::values::category::Category;
use openintel::domain::values::trade_direction::TradeDirection;
use openintel::domain::values::trade_outcome::TradeOutcome;
use openintel::OpenIntel;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let db_path = std::env::var("OPENINTEL_DB").unwrap_or_else(|_| "./openintel.db".into());

    let oi = match OpenIntel::new(&db_path) {
        Ok(oi) => oi,
        Err(e) => {
            eprintln!("Error initializing OpenIntel: {e}");
            std::process::exit(1);
        }
    };

    let result = run_command(oi, cli.command).await;
    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

async fn run_command(oi: OpenIntel, cmd: Commands) -> Result<(), Box<dyn std::error::Error>> {
    match cmd {
        Commands::Add { category, json } => {
            let cat: Category = category.parse().map_err(|e: String| e)?;
            let data: serde_json::Value = serde_json::from_str(&json)?;

            let title = data["title"]
                .as_str()
                .ok_or("Missing required field: title")?
                .to_string();
            let body = data["body"]
                .as_str()
                .ok_or("Missing required field: body")?
                .to_string();
            let source = data["source"].as_str().map(|s| s.to_string());
            let tags: Vec<String> = data["tags"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let confidence = data["confidence"].as_f64();
            let actionable = data["actionable"].as_bool();
            let metadata = data.get("metadata").cloned();

            let entry = oi
                .add_intel(
                    cat, title, body, source, tags, confidence, actionable, metadata,
                )
                .await?;
            println!("{}", serde_json::to_string_pretty(&entry).unwrap());
        }
        Commands::Query {
            category,
            limit,
            since,
            tag,
        } => {
            let cat: Category = category.parse().map_err(|e: String| e)?;
            let since_dt = parse_date(&since)?;
            let entries = oi.query(Some(cat), tag, since_dt, Some(limit))?;
            println!("{}", serde_json::to_string_pretty(&entries).unwrap());
        }
        Commands::Search { text, limit } => {
            let entries = oi.keyword_search(&text, limit)?;
            println!("{}", serde_json::to_string_pretty(&entries).unwrap());
        }
        Commands::Semantic { query, limit } => {
            let entries = oi.semantic_search(&query, limit).await?;
            println!("{}", serde_json::to_string_pretty(&entries).unwrap());
        }
        Commands::Think { query, limit } => {
            let entries = oi.hybrid_search(&query, limit).await?;
            println!("{}", serde_json::to_string_pretty(&entries).unwrap());
        }
        Commands::Stats => {
            let stats = oi.stats()?;
            println!("{}", serde_json::to_string_pretty(&stats).unwrap());
        }
        Commands::Tags { category } => {
            let cat = category
                .map(|c| c.parse())
                .transpose()
                .map_err(|e: String| e)?;
            let tags = oi.tags(cat)?;
            for t in &tags {
                println!("{}: {}", t.tag, t.count);
            }
        }
        Commands::TradeAdd { json } => {
            let data: serde_json::Value = serde_json::from_str(&json)?;

            let ticker = data["ticker"]
                .as_str()
                .ok_or("ticker required")?
                .to_string();
            let series_ticker = data["series_ticker"].as_str().map(String::from);
            let direction: TradeDirection = data["direction"]
                .as_str()
                .ok_or("direction required")?
                .parse()
                .map_err(|e: String| e)?;
            let contracts = data["contracts"].as_i64().ok_or("contracts required")?;
            let entry_price = data["entry_price"].as_f64().ok_or("entry_price required")?;
            let thesis = data["thesis"].as_str().map(String::from);

            let trade = oi.trade_add(
                ticker,
                series_ticker,
                direction,
                contracts,
                entry_price,
                thesis,
            )?;
            println!("{}", serde_json::to_string_pretty(&trade).unwrap());
        }
        Commands::TradeResolve {
            id,
            outcome,
            pnl_cents,
            exit_price,
        } => {
            let out: TradeOutcome = outcome.parse().map_err(|e: String| e)?;
            oi.trade_resolve(&id, out, pnl_cents, exit_price)?;
            println!("Trade {id} resolved as {outcome} ({pnl_cents} cents)");
        }
        Commands::Trades {
            limit,
            since,
            resolved,
        } => {
            let since_dt = parse_date(&since)?;
            let trades = oi.trade_list(Some(limit), since_dt, resolved)?;
            println!("{}", serde_json::to_string_pretty(&trades).unwrap());
        }
        Commands::Export { since, category } => {
            let since_dt = parse_date(&since)?;
            let cat = category
                .map(|c| c.parse())
                .transpose()
                .map_err(|e: String| e)?;
            let entries = oi.query(cat, None, since_dt, None)?;
            println!("{}", serde_json::to_string_pretty(&entries).unwrap());
        }
        Commands::Reindex => {
            let count = oi.reindex().await?;
            println!("Reindexed {count} entries");
        }
    }
    Ok(())
}

fn parse_date(s: &Option<String>) -> Result<Option<chrono::DateTime<chrono::Utc>>, String> {
    match s {
        None => Ok(None),
        Some(s) => {
            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
                return Ok(Some(dt.with_timezone(&chrono::Utc)));
            }
            if let Ok(date) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
                let dt = date.and_hms_opt(0, 0, 0).unwrap();
                return Ok(Some(chrono::DateTime::from_naive_utc_and_offset(
                    dt,
                    chrono::Utc,
                )));
            }
            Err(format!(
                "Invalid date format: {s}. Use YYYY-MM-DD or RFC3339"
            ))
        }
    }
}
