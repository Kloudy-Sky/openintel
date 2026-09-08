//! CLI leaf for `openintel watch`: the long-running poll loop. Unlike the
//! other leaves it prints as it goes — one JSON line per event to stdout,
//! errors to stderr — because the process is the stream.

use std::time::Duration;

use chrono::Utc;
use secrecy::SecretString;

use crate::adapters::filings::edgar::EdgarSource;
use crate::adapters::market::yahoo::YahooMarketSource;
use crate::adapters::notify::ntfy::NtfyNotifier;
use crate::application::chatter::{ChatterRequest, DEFAULT_FREE_LIMIT};
use crate::application::watch::{
    append_events, default_events_path, poll, poll_chatter, PollOutcome, WatchCache, WatchDeps,
    WatchRequest, FRAMING,
};
use crate::cli::args::WatchArgs;
use crate::config::secrets::Credentials;
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::ports::listening_feed::ListeningFeed;
use crate::domain::ports::notifier::Notifier;
use crate::domain::values::event::Event;
use crate::domain::watch::WatchState;

const MIN_INTERVAL_SECS: u64 = 15;
const MAX_INTERVAL_SECS: u64 = 3600;

fn fail(message: impl Into<String>) -> DomainError {
    DomainError::SourceFailure {
        name: "watch".into(),
        message: message.into(),
    }
}

/// Open journal positions plus the tickers passed, deduplicated in order.
fn watched_tickers(args: &WatchArgs) -> Result<Vec<Ticker>, DomainError> {
    let mut raw: Vec<String> = Vec::new();
    if let Some(path) = crate::application::trade_journal::default_path() {
        if path.exists() {
            for trade in crate::application::trade_journal::open_positions(&path).open {
                raw.push(trade.instrument.symbol().to_string());
            }
        }
    }
    raw.extend(args.tickers.iter().cloned());
    let mut tickers: Vec<Ticker> = Vec::new();
    for r in raw {
        let t = Ticker::parse(&r)?;
        if !tickers.contains(&t) {
            tickers.push(t);
        }
    }
    if tickers.is_empty() {
        return Err(fail(
            "nothing to watch: pass --tickers or log an open position first",
        ));
    }
    Ok(tickers)
}

fn notifier_from(args: &WatchArgs) -> Result<Option<NtfyNotifier>, DomainError> {
    let topic = args
        .ntfy_topic
        .clone()
        .or_else(|| std::env::var("OPENINTEL_NTFY_TOPIC").ok());
    topic
        .map(|t| NtfyNotifier::new(SecretString::from(t)))
        .transpose()
}

async fn emit(
    outcome: &PollOutcome,
    events_path: Option<&std::path::Path>,
    notifier: Option<&NtfyNotifier>,
) {
    for event in &outcome.events {
        match serde_json::to_string(event) {
            Ok(line) => println!("{line}"),
            Err(e) => eprintln!("watch: could not render an event: {e}"),
        }
    }
    if let Some(path) = events_path {
        if let Err(e) = append_events(path, &outcome.events) {
            eprintln!("watch: {e}");
        }
    }
    if let Some(n) = notifier {
        for event in &outcome.events {
            if let Err(e) = n.notify(&event.summary, &push_body(event)).await {
                eprintln!("watch: push failed: {e}");
            }
        }
    }
    for e in &outcome.errors {
        eprintln!("watch: {e}");
    }
}

fn push_body(event: &Event) -> String {
    let mut body = event.evidence.join("\n");
    if !body.is_empty() {
        body.push('\n');
    }
    body.push_str(&format!(
        "polled {} · {}",
        event.polled_at.format("%H:%M:%SZ"),
        event.source
    ));
    body
}

pub async fn run(args: &WatchArgs, credentials: &Credentials) -> Result<(), DomainError> {
    let interval = args.interval.clamp(MIN_INTERVAL_SECS, MAX_INTERVAL_SECS);
    let tickers = watched_tickers(args)?;
    let notifier = notifier_from(args)?;
    let events_path = default_events_path();

    let yahoo = YahooMarketSource::new()?;
    let edgar = EdgarSource::new()?;
    let deps = WatchDeps {
        bars: &yahoo,
        market: &yahoo,
        news: &yahoo,
        filings: &edgar,
    };
    let req = WatchRequest {
        tickers,
        move_threshold_atr: args.move_atr,
    };

    let listening_path = crate::config::listening::default_path()
        .ok_or_else(|| fail("cannot resolve a home directory"))?;
    let (listening, _) = crate::config::listening::load_or_seed(&listening_path)?;
    let free = crate::adapters::sources::build_free_listening_feeds(credentials);
    let feeds: Vec<&dyn ListeningFeed> = free.iter().map(|f| f.as_ref()).collect();
    let chatter_req = ChatterRequest {
        listening,
        hours: 24,
        free_limit: DEFAULT_FREE_LIMIT,
        x_read_cap: 1,
        baseline_path: crate::application::chatter::default_baseline_path(),
        write_baseline: false,
    };
    let chatter_every =
        (args.chatter_every > 0).then(|| chrono::Duration::minutes(args.chatter_every as i64));

    eprintln!(
        "watch: {} names every {interval}s{}{} · {}",
        req.tickers.len(),
        chatter_every
            .map(|d| format!(" · chatter every {}m", d.num_minutes()))
            .unwrap_or_default(),
        notifier
            .as_ref()
            .map(|_| " · pushing to ntfy")
            .unwrap_or_default(),
        FRAMING
    );

    let mut state = WatchState::default();
    let mut cache = WatchCache::default();
    let mut last_chatter: Option<chrono::DateTime<Utc>> = None;
    let mut timer = tokio::time::interval(Duration::from_secs(interval));
    loop {
        timer.tick().await;
        let now = Utc::now();
        let outcome = poll(&req, &deps, &mut state, &mut cache, now).await;
        emit(&outcome, events_path.as_deref(), notifier.as_ref()).await;

        let chatter_due = match (chatter_every, last_chatter) {
            (Some(_), None) => true,
            (Some(every), Some(last)) => now - last >= every,
            (None, _) => false,
        };
        if chatter_due && !feeds.is_empty() {
            last_chatter = Some(now);
            let outcome = poll_chatter(&chatter_req, &feeds, Some(&yahoo), &mut state, now).await;
            emit(&outcome, events_path.as_deref(), notifier.as_ref()).await;
        }
        if args.once {
            eprintln!(
                "watch: polled once · {} events · {} errors",
                outcome.events.len(),
                outcome.errors.len()
            );
            return Ok(());
        }
    }
}
