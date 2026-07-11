//! `openintel setup <source>` — guided, env-only credential setup + live verify.
//!
//! Never stores or writes credentials (see SECURITY.md): it only reads
//! `Credentials::from_env()` and tells the user what to do next. This is the
//! one CLI-leaf module that prints to stdout directly — it IS the user-facing
//! output, and it never runs under the MCP stdio server.

use std::process::ExitCode;

use secrecy::SecretString;

use crate::adapters::sources::bluesky::BlueskySource;
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
        SetupSource::Bluesky => setup_bluesky(credentials).await,
    }
}

/// Which of the three setup modes applies, given which env vars are set.
/// First/second = the source's (identifier-like, secret-like) credential pair
/// — (client id, client secret) for Reddit; (handle, app password) for Bluesky.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Verify,
    MissingFirst,
    MissingSecond,
    Guide,
}

fn plan(first_set: bool, second_set: bool) -> Mode {
    match (first_set, second_set) {
        (true, true) => Mode::Verify,
        (false, true) => Mode::MissingFirst,
        (true, false) => Mode::MissingSecond,
        (false, false) => Mode::Guide,
    }
}

async fn setup_reddit(credentials: &Credentials) -> ExitCode {
    match plan(
        credentials.reddit_client_id.is_some(),
        credentials.reddit_client_secret.is_some(),
    ) {
        Mode::Guide => {
            println!("{}", reddit_guide_text());
            ExitCode::FAILURE
        }
        Mode::MissingFirst => {
            println!("{}", partial_text("Reddit", "OPENINTEL_REDDIT_CLIENT_ID"));
            ExitCode::FAILURE
        }
        Mode::MissingSecond => {
            println!(
                "{}",
                partial_text("Reddit", "OPENINTEL_REDDIT_CLIENT_SECRET")
            );
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
            match probe_reddit(id, secret).await {
                Ok(count) => {
                    println!(
                        "{}",
                        verify_ok_text("Reddit", count, "openintel analyze GME --enable-reddit")
                    );
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    println!("{}", verify_err_text(&e, REDDIT_UNAUTHORIZED_HINT));
                    ExitCode::FAILURE
                }
            }
        }
    }
}

/// One live round trip through the full Reddit path: OAuth token request plus
/// a search. Returns how many posts the test query yielded.
async fn probe_reddit(id: SecretString, secret: SecretString) -> Result<usize, DomainError> {
    let source = RedditSource::new(id, secret)?;
    let ticker = Ticker::parse("AAPL")?;
    let posts = source.fetch(&ticker, 1).await?;
    Ok(posts.len())
}

fn reddit_guide_text() -> String {
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

fn partial_text(source_label: &str, missing: &str) -> String {
    format!(
        "⚠  {source_label} is half-configured: {missing} is not set.\n   \
         Set it, then re-run. (Run `openintel setup {}` with neither variable\n   \
         set to see the full setup guide.)",
        source_label.to_lowercase()
    )
}

fn verify_ok_text(source_label: &str, count: usize, try_cmd: &str) -> String {
    let evidence = if count > 0 {
        format!("pulled {count} recent post(s) for a test query")
    } else {
        "credentials work — the test query just had no recent posts, which is fine".to_string()
    };
    format!(
        "✅ {source_label} is configured and working ({evidence}).\n   \
         Real {source_label} sentiment is active. Try:  {try_cmd}"
    )
}

fn verify_err_text(err: &DomainError, unauthorized_hint: &str) -> String {
    let msg = err.to_string();
    let hint = if msg.contains("unauthorized") {
        unauthorized_hint
    } else if msg.contains("rate limited") {
        "You're being rate-limited right now — wait a minute and re-run."
    } else {
        "Check your internet connection and try again."
    };
    format!("❌ {msg}\n   {hint}")
}

const REDDIT_UNAUTHORIZED_HINT: &str =
    "Your client id or secret looks wrong. Re-copy both from\n   \
         https://www.reddit.com/prefs/apps (the id is the short string under the app\n   \
         name; the secret is labelled \"secret\").";

const BLUESKY_UNAUTHORIZED_HINT: &str =
    "Your handle or app password looks wrong. Check the handle\n   \
         (e.g. yourname.bsky.social) and generate a fresh app password at\n   \
         https://bsky.app/settings/app-passwords (the value is shown only once).";

async fn setup_bluesky(credentials: &Credentials) -> ExitCode {
    match plan(
        credentials.bluesky_handle.is_some(),
        credentials.bluesky_app_password.is_some(),
    ) {
        Mode::Guide => {
            println!("{}", bluesky_guide_text());
            ExitCode::FAILURE
        }
        Mode::MissingFirst => {
            println!("{}", partial_text("Bluesky", "OPENINTEL_BLUESKY_HANDLE"));
            ExitCode::FAILURE
        }
        Mode::MissingSecond => {
            println!(
                "{}",
                partial_text("Bluesky", "OPENINTEL_BLUESKY_APP_PASSWORD")
            );
            ExitCode::FAILURE
        }
        Mode::Verify => {
            println!("Checking your Bluesky credentials…");
            let (Some(handle), Some(password)) = (
                credentials.bluesky_handle.clone(),
                credentials.bluesky_app_password.clone(),
            ) else {
                // Unreachable: Mode::Verify is returned only when both are set.
                println!("internal error: credentials unavailable");
                return ExitCode::FAILURE;
            };
            match probe_bluesky(handle, password).await {
                Ok(count) => {
                    println!(
                        "{}",
                        verify_ok_text("Bluesky", count, "openintel analyze GME --enable-bluesky")
                    );
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    println!("{}", verify_err_text(&e, BLUESKY_UNAUTHORIZED_HINT));
                    ExitCode::FAILURE
                }
            }
        }
    }
}

/// One live round trip through the full Bluesky path: createSession plus a search.
/// Returns how many posts the test query yielded.
async fn probe_bluesky(handle: String, password: SecretString) -> Result<usize, DomainError> {
    let source = BlueskySource::new(handle, password)?;
    let ticker = Ticker::parse("AAPL")?;
    let posts = source.fetch(&ticker, 1).await?;
    Ok(posts.len())
}

fn bluesky_guide_text() -> String {
    "\
Bluesky needs a free app password — search requires auth. ~2 minutes:

  1. Create a free account at https://bsky.app if you don't have one.
  2. Sign in, then open:  https://bsky.app/settings/app-passwords
     (Settings → Privacy and Security → App Passwords).
  3. Click \"Add App Password\", name it  openintel , and copy the generated
     password — it is shown only once (format: xxxx-xxxx-xxxx-xxxx).
  4. Put your handle and the app password in your shell (or a gitignored
     .env — see .env.example), then re-run this command:

       export OPENINTEL_BLUESKY_HANDLE=yourname.bsky.social
       export OPENINTEL_BLUESKY_APP_PASSWORD=xxxx-xxxx-xxxx-xxxx
       openintel setup bluesky

openintel reads these only from your environment — it never stores or writes
your credentials to disk."
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_selects_mode_for_all_credential_combinations() {
        assert_eq!(plan(true, true), Mode::Verify);
        assert_eq!(plan(false, true), Mode::MissingFirst);
        assert_eq!(plan(true, false), Mode::MissingSecond);
        assert_eq!(plan(false, false), Mode::Guide);
    }

    #[test]
    fn reddit_guide_text_contains_every_load_bearing_instruction() {
        let text = reddit_guide_text();
        assert!(text.contains("https://www.reddit.com/prefs/apps"));
        assert!(text.contains("\"script\""));
        assert!(text.contains("export OPENINTEL_REDDIT_CLIENT_ID="));
        assert!(text.contains("export OPENINTEL_REDDIT_CLIENT_SECRET="));
        assert!(text.contains("never stores"));
    }

    #[test]
    fn partial_text_names_the_missing_variable() {
        assert!(partial_text("Reddit", "OPENINTEL_REDDIT_CLIENT_ID")
            .contains("OPENINTEL_REDDIT_CLIENT_ID is not set"));
        assert!(partial_text("Reddit", "OPENINTEL_REDDIT_CLIENT_SECRET")
            .contains("OPENINTEL_REDDIT_CLIENT_SECRET is not set"));
    }

    #[test]
    fn verify_ok_text_distinguishes_empty_from_nonempty_results() {
        let some = verify_ok_text("Reddit", 3, "openintel analyze GME --enable-reddit");
        assert!(some.contains("pulled 3 recent post(s)"));
        let none = verify_ok_text("Reddit", 0, "openintel analyze GME --enable-reddit");
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
        assert!(verify_err_text(&unauthorized, REDDIT_UNAUTHORIZED_HINT).contains("Re-copy both"));

        let rate_limited = DomainError::SourceFailure {
            name: "reddit".into(),
            message: "rate limited (HTTP 429)".into(),
        };
        assert!(verify_err_text(&rate_limited, REDDIT_UNAUTHORIZED_HINT).contains("wait a minute"));

        let other = DomainError::SourceFailure {
            name: "reddit".into(),
            message: "search request failed: connection refused".into(),
        };
        let text = verify_err_text(&other, REDDIT_UNAUTHORIZED_HINT);
        assert!(text.contains("connection refused")); // raw error preserved
        assert!(text.contains("Check your internet connection"));
    }

    #[test]
    fn bluesky_guide_text_contains_every_load_bearing_instruction() {
        let text = bluesky_guide_text();
        assert!(text.contains("https://bsky.app/settings/app-passwords"));
        assert!(text.contains("Add App Password"));
        assert!(text.contains("export OPENINTEL_BLUESKY_HANDLE="));
        assert!(text.contains("export OPENINTEL_BLUESKY_APP_PASSWORD="));
        assert!(text.contains("never stores"));
    }

    #[test]
    fn partial_text_is_source_aware() {
        let text = partial_text("Bluesky", "OPENINTEL_BLUESKY_APP_PASSWORD");
        assert!(text.contains("Bluesky is half-configured"));
        assert!(text.contains("OPENINTEL_BLUESKY_APP_PASSWORD is not set"));
        assert!(text.contains("openintel setup bluesky"));
    }

    #[test]
    fn verify_err_text_uses_per_source_unauthorized_hint() {
        let unauthorized = DomainError::SourceFailure {
            name: "bluesky".into(),
            message: "unauthorized — check handle/app password".into(),
        };
        let text = verify_err_text(&unauthorized, BLUESKY_UNAUTHORIZED_HINT);
        assert!(text.contains("app-passwords"));
        assert!(!text.contains("prefs/apps"));
    }
}
