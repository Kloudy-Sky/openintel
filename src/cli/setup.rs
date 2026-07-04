//! `openintel setup <source>` — guided, env-only credential setup + live verify.
//!
//! Never stores or writes credentials (see SECURITY.md): it only reads
//! `Credentials::from_env()` and tells the user what to do next. This is the
//! one CLI-leaf module that prints to stdout directly — it IS the user-facing
//! output, and it never runs under the MCP stdio server.

use std::process::ExitCode;

use secrecy::SecretString;

use crate::adapters::sources::reddit::RedditSource;
use crate::cli::args::SetupSource;
use crate::config::secrets::Credentials;
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::ports::social_data_source::SocialDataSource;

/// Entry point for `openintel setup <source>`. Exit code 0 only when the
/// source is fully configured and a live probe succeeds.
pub async fn run(source: SetupSource, credentials: &Credentials) -> ExitCode {
    match source {
        SetupSource::Reddit => setup_reddit(credentials).await,
    }
}

/// Which of the three setup modes applies, given which env vars are set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Verify,
    MissingId,
    MissingSecret,
    Guide,
}

fn plan(id_set: bool, secret_set: bool) -> Mode {
    match (id_set, secret_set) {
        (true, true) => Mode::Verify,
        (false, true) => Mode::MissingId,
        (true, false) => Mode::MissingSecret,
        (false, false) => Mode::Guide,
    }
}

async fn setup_reddit(credentials: &Credentials) -> ExitCode {
    match plan(
        credentials.reddit_client_id.is_some(),
        credentials.reddit_client_secret.is_some(),
    ) {
        Mode::Guide => {
            println!("{}", guide_text());
            ExitCode::FAILURE
        }
        Mode::MissingId => {
            println!("{}", partial_text("OPENINTEL_REDDIT_CLIENT_ID"));
            ExitCode::FAILURE
        }
        Mode::MissingSecret => {
            println!("{}", partial_text("OPENINTEL_REDDIT_CLIENT_SECRET"));
            ExitCode::FAILURE
        }
        Mode::Verify => {
            println!("Checking your Reddit credentials…");
            let (Some(id), Some(secret)) = (
                credentials.reddit_client_id.clone(),
                credentials.reddit_client_secret.clone(),
            ) else {
                // Unreachable: Mode::Verify is returned only when both are set.
                println!("internal error: credentials unavailable");
                return ExitCode::FAILURE;
            };
            match probe(id, secret).await {
                Ok(count) => {
                    println!("{}", verify_ok_text(count));
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    println!("{}", verify_err_text(&e));
                    ExitCode::FAILURE
                }
            }
        }
    }
}

/// One live round trip through the full Reddit path: OAuth token request plus
/// a search. Returns how many posts the test query yielded.
async fn probe(id: SecretString, secret: SecretString) -> Result<usize, DomainError> {
    let source = RedditSource::new(id, secret)?;
    let ticker = Ticker::parse("AAPL")?;
    let posts = source.fetch(&ticker, 1).await?;
    Ok(posts.len())
}

fn guide_text() -> String {
    "\
Reddit needs a free OAuth app — there's no keyless access. ~2 minutes:

  1. Sign in to Reddit, then open:  https://www.reddit.com/prefs/apps
  2. Scroll to the bottom and click \"create another app…\"
     (or \"are you a developer? create an app…\").
  3. Fill in the form:
       • name           openintel        (anything is fine)
       • type           select \"script\"  ← this matters
       • redirect uri   http://localhost:8080   (unused, but required)
     Click \"create app\".
  4. On the app that appears:
       • CLIENT ID  — the short string just under the app name
                      (below \"personal use script\")
       • SECRET     — the value labelled \"secret\"
  5. Put them in your shell (or a gitignored .env — see .env.example), then
     re-run this command:

       export OPENINTEL_REDDIT_CLIENT_ID=paste_your_client_id
       export OPENINTEL_REDDIT_CLIENT_SECRET=paste_your_secret
       openintel setup reddit

openintel reads these only from your environment — it never stores or writes
your credentials to disk."
        .to_string()
}

fn partial_text(missing: &str) -> String {
    format!(
        "⚠  Reddit is half-configured: {missing} is not set.\n   \
         Set it, then re-run. (Run `openintel setup reddit` with neither variable\n   \
         set to see the full setup guide.)"
    )
}

fn verify_ok_text(count: usize) -> String {
    let evidence = if count > 0 {
        format!("pulled {count} recent post(s) for a test query")
    } else {
        "credentials work — the test query just had no recent posts, which is fine".to_string()
    };
    format!(
        "✅ Reddit is configured and working ({evidence}).\n   \
         Real Reddit sentiment is active. Try:  openintel analyze GME --enable-reddit"
    )
}

fn verify_err_text(err: &DomainError) -> String {
    let msg = err.to_string();
    let hint = if msg.contains("unauthorized") {
        "Your client id or secret looks wrong. Re-copy both from\n   \
         https://www.reddit.com/prefs/apps (the id is the short string under the app\n   \
         name; the secret is labelled \"secret\")."
    } else if msg.contains("rate limited") {
        "Reddit is rate-limiting right now — wait a minute and re-run."
    } else {
        "Check your internet connection and try again."
    };
    format!("❌ {msg}\n   {hint}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_selects_mode_for_all_credential_combinations() {
        assert_eq!(plan(true, true), Mode::Verify);
        assert_eq!(plan(false, true), Mode::MissingId);
        assert_eq!(plan(true, false), Mode::MissingSecret);
        assert_eq!(plan(false, false), Mode::Guide);
    }

    #[test]
    fn guide_text_contains_every_load_bearing_instruction() {
        let text = guide_text();
        assert!(text.contains("https://www.reddit.com/prefs/apps"));
        assert!(text.contains("\"script\""));
        assert!(text.contains("export OPENINTEL_REDDIT_CLIENT_ID="));
        assert!(text.contains("export OPENINTEL_REDDIT_CLIENT_SECRET="));
        assert!(text.contains("never stores"));
    }

    #[test]
    fn partial_text_names_the_missing_variable() {
        assert!(partial_text("OPENINTEL_REDDIT_CLIENT_ID")
            .contains("OPENINTEL_REDDIT_CLIENT_ID is not set"));
        assert!(partial_text("OPENINTEL_REDDIT_CLIENT_SECRET")
            .contains("OPENINTEL_REDDIT_CLIENT_SECRET is not set"));
    }

    #[test]
    fn verify_ok_text_distinguishes_empty_from_nonempty_results() {
        let some = verify_ok_text(3);
        assert!(some.contains("pulled 3 recent post(s)"));
        let none = verify_ok_text(0);
        assert!(none.contains("no recent posts"));
        for text in [&some, &none] {
            assert!(text.contains("✅ Reddit is configured and working"));
            assert!(text.contains("openintel analyze GME --enable-reddit"));
        }
    }

    #[test]
    fn verify_err_text_maps_known_failures_to_hints() {
        let unauthorized = DomainError::SourceFailure {
            name: "reddit".into(),
            message: "unauthorized — check client id/secret".into(),
        };
        assert!(verify_err_text(&unauthorized).contains("Re-copy both"));

        let rate_limited = DomainError::SourceFailure {
            name: "reddit".into(),
            message: "rate limited (HTTP 429)".into(),
        };
        assert!(verify_err_text(&rate_limited).contains("wait a minute"));

        let other = DomainError::SourceFailure {
            name: "reddit".into(),
            message: "search request failed: connection refused".into(),
        };
        let text = verify_err_text(&other);
        assert!(text.contains("connection refused")); // raw error preserved
        assert!(text.contains("Check your internet connection"));
    }
}
