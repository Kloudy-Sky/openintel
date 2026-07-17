//! `openintel setup <source>` — guided credential setup + live verify.
//!
//! Interactive (TTY): condensed guide -> prompts (secret input hidden) ->
//! live verify -> save to the OS keychain (only after ✅; env vars always
//! override). Non-TTY keeps the classic guide/partial/verify behavior.
//! This is the one CLI-leaf module that prints to stdout directly — it IS
//! the user-facing output, and it never runs under the MCP stdio server.

use std::io::{BufRead, IsTerminal, Write};
use std::process::ExitCode;

use secrecy::SecretString;

use crate::adapters::sources::bluesky::BlueskySource;
use crate::adapters::sources::reddit::RedditSource;
use crate::adapters::sources::x::XPulseSource;
use crate::cli::args::SetupSource;
use crate::config::secrets::Credentials;
use crate::config::store::CredentialStore;
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::ports::influencer_feed::InfluencerFeed;
use crate::domain::ports::social_data_source::SocialDataSource;

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
    } else if msg.contains("forbidden") {
        "Your token authenticated but access was refused — most often exhausted\n   \
         API credits. Check Billing → Credits in the X developer console."
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

/// Everything the shared interactive loop needs to know about one source.
struct SourceSpec {
    label: &'static str,
    first_key: &'static str,
    second_key: Option<&'static str>,
    first_prompt: &'static str,
    second_prompt: Option<&'static str>,
    condensed_guide: &'static str,
    unauthorized_hint: &'static str,
    try_cmd: &'static str,
    /// Shown (and requires y/blank to proceed) right before the live probe —
    /// for paid sources where verification itself costs money.
    pre_probe_confirm: Option<&'static str>,
}

const REDDIT_SPEC: SourceSpec = SourceSpec {
    label: "Reddit",
    first_key: "OPENINTEL_REDDIT_CLIENT_ID",
    second_key: Some("OPENINTEL_REDDIT_CLIENT_SECRET"),
    first_prompt: "Client id",
    second_prompt: Some("Client secret"),
    pre_probe_confirm: None,
    condensed_guide: "\
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
  5. Enter them below — they'll be verified live and saved to your OS
     keychain (plaintext never touches disk).
",
    unauthorized_hint: REDDIT_UNAUTHORIZED_HINT,
    try_cmd: "openintel analyze GME --enable-reddit",
};

const BLUESKY_SPEC: SourceSpec = SourceSpec {
    label: "Bluesky",
    first_key: "OPENINTEL_BLUESKY_HANDLE",
    second_key: Some("OPENINTEL_BLUESKY_APP_PASSWORD"),
    first_prompt: "Handle (e.g. yourname.bsky.social)",
    second_prompt: Some("App password"),
    pre_probe_confirm: None,
    condensed_guide: "\
Bluesky needs a free app password — search requires auth. ~2 minutes:

  1. Create a free account at https://bsky.app if you don't have one.
  2. Sign in, then open:  https://bsky.app/settings/app-passwords
     (Settings → Privacy and Security → App Passwords).
  3. Click \"Add App Password\", name it  openintel , and copy the generated
     password — it is shown only once (format: xxxx-xxxx-xxxx-xxxx).
  4. Enter your handle and the app password below — they'll be verified live
     and saved to your OS keychain (plaintext never touches disk).
",
    unauthorized_hint: BLUESKY_UNAUTHORIZED_HINT,
    try_cmd: "openintel analyze GME --enable-bluesky",
};

const X_UNAUTHORIZED_HINT: &str =
    "Your bearer token looks wrong or lacks access. In the X developer console\n   \
     (https://developer.x.com → Projects & Apps → your app → Keys and Tokens),\n   \
     regenerate the Bearer Token, and make sure API credits are loaded.";

const X_SPEC: SourceSpec = SourceSpec {
    label: "X",
    first_key: "OPENINTEL_X_BEARER",
    second_key: None,
    first_prompt: "Bearer token",
    second_prompt: None,
    condensed_guide: "\
X Pulse needs an X API bearer token — the API is PAID (pay-per-use,
about $0.005 per post read; you buy credits up front). ~3 minutes:

  1. Sign in at https://developer.x.com and open the developer console.
  2. Projects & Apps → your app (create one if needed) → Keys and Tokens.
  3. Under \"Bearer Token\", generate/reveal it and copy it.
  4. Make sure your account has API credits loaded (Billing → Credits).
  5. Enter the token below — verification reads up to 10 posts (≈ $0.05).
",
    unauthorized_hint: X_UNAUTHORIZED_HINT,
    try_cmd: "openintel pulse NVDA --accounts jensenhuang",
    pre_probe_confirm: Some(
        "Verifying will read up to 10 posts from X (≈ $0.05). Proceed? [Y/n]: ",
    ),
};

/// Entry point for `openintel setup <source>`. Exit code 0 only when the
/// source is verified working (or `--forget` succeeded).
pub async fn run(
    source: SetupSource,
    credentials: &Credentials,
    store: &dyn CredentialStore,
    forget: bool,
) -> ExitCode {
    let spec = match source {
        SetupSource::Reddit => &REDDIT_SPEC,
        SetupSource::Bluesky => &BLUESKY_SPEC,
        SetupSource::X => &X_SPEC,
    };

    if forget {
        return forget_source(spec, store);
    }

    if !std::io::stdin().is_terminal() {
        // Piped / CI: the classic guide/partial/verify behavior, unchanged.
        return match source {
            SetupSource::Reddit => setup_reddit(credentials).await,
            SetupSource::Bluesky => setup_bluesky(credentials).await,
            SetupSource::X => setup_x(credentials).await,
        };
    }

    let configured = match source {
        SetupSource::Reddit => {
            credentials.reddit_client_id.is_some() && credentials.reddit_client_secret.is_some()
        }
        SetupSource::Bluesky => {
            credentials.bluesky_handle.is_some() && credentials.bluesky_app_password.is_some()
        }
        SetupSource::X => credentials.x_bearer.is_some(),
    };
    let already = configured.then(|| provenance(spec.first_key));

    let mut stdin = std::io::stdin().lock();
    let read_secret = |prompt: &str| {
        rpassword::prompt_password(prompt).map(|s| SecretString::new(s.into_boxed_str()))
    };
    let mut io = SetupIo {
        input: &mut stdin,
        read_secret: &read_secret,
    };

    let outcome = match source {
        SetupSource::Reddit => {
            run_interactive(&mut io, store, spec, already, |first, secret| {
                probe_reddit(SecretString::new(first.into_boxed_str()), secret)
            })
            .await
        }
        SetupSource::Bluesky => run_interactive(&mut io, store, spec, already, probe_bluesky).await,
        SetupSource::X => {
            run_interactive(&mut io, store, spec, already, |_first, secret| {
                probe_x(secret)
            })
            .await
        }
    };

    match outcome {
        InteractiveOutcome::Done(Outcome::Success) => ExitCode::SUCCESS,
        InteractiveOutcome::Done(Outcome::Failure) => ExitCode::FAILURE,
        InteractiveOutcome::VerifyExisting => {
            if let Some(confirm) = spec.pre_probe_confirm {
                match read_visible(&mut io, confirm) {
                    Ok(ans) if confirm_answer_proceeds(&ans) => {}
                    Ok(_) => {
                        println!("Aborted — nothing spent.");
                        return ExitCode::FAILURE;
                    }
                    Err(_) => return ExitCode::FAILURE,
                }
            }
            match source {
                SetupSource::Reddit => setup_reddit(credentials).await,
                SetupSource::Bluesky => setup_bluesky(credentials).await,
                SetupSource::X => setup_x(credentials).await,
            }
        }
    }
}

/// Where the already-configured credentials came from, for the replace-ask.
fn provenance(first_key: &str) -> &'static str {
    if std::env::var(first_key)
        .map(|v| !v.is_empty())
        .unwrap_or(false)
    {
        "the environment"
    } else {
        "the OS keychain"
    }
}

fn forget_source(spec: &SourceSpec, store: &dyn CredentialStore) -> ExitCode {
    match forget_outcome(spec, store) {
        Outcome::Success => ExitCode::SUCCESS,
        Outcome::Failure => ExitCode::FAILURE,
    }
}

fn forget_outcome(spec: &SourceSpec, store: &dyn CredentialStore) -> Outcome {
    for key in [Some(spec.first_key), spec.second_key]
        .into_iter()
        .flatten()
    {
        if let Err(e) = store.delete(key) {
            println!("❌ could not remove {key} from the OS keychain: {e}");
            return Outcome::Failure;
        }
    }
    println!(
        "Removed {} credentials from the OS keychain. (Env vars, if set, still apply.)",
        spec.label
    );
    Outcome::Success
}

/// Injected I/O so the interactive loop is unit-testable without a TTY.
struct SetupIo<'a> {
    input: &'a mut dyn BufRead,
    read_secret: &'a dyn Fn(&str) -> std::io::Result<SecretString>,
}

/// `std::process::ExitCode` has no `PartialEq`, so the loop reports this
/// testable outcome and `run()` maps it to an exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Success,
    Failure,
}

enum InteractiveOutcome {
    Done(Outcome),
    /// User declined to replace existing creds -> caller runs the classic verify.
    VerifyExisting,
}

const MAX_ATTEMPTS: usize = 3;

/// Shared decision for every "cost confirm" prompt (blank/y/yes -> proceed).
fn confirm_answer_proceeds(ans: &str) -> bool {
    matches!(ans.trim().to_lowercase().as_str(), "" | "y" | "yes")
}

async fn run_interactive<F, Fut>(
    io: &mut SetupIo<'_>,
    store: &dyn CredentialStore,
    spec: &SourceSpec,
    already_configured_from: Option<&'static str>,
    probe: F,
) -> InteractiveOutcome
where
    F: Fn(String, SecretString) -> Fut,
    Fut: std::future::Future<Output = Result<usize, DomainError>>,
{
    if let Some(source_of_truth) = already_configured_from {
        println!(
            "{} is already configured (from {source_of_truth}).",
            spec.label
        );
        match read_visible(io, "Replace it? [y/N]: ") {
            Ok(ans) if matches!(ans.trim().to_lowercase().as_str(), "y" | "yes") => {}
            Ok(_) => return InteractiveOutcome::VerifyExisting,
            Err(_) => return InteractiveOutcome::Done(Outcome::Failure),
        }
    }

    println!("{}", spec.condensed_guide);

    for attempt in 1..=MAX_ATTEMPTS {
        let (first, secret) = if let Some(second_prompt) = spec.second_prompt {
            let Ok(first) = prompt_nonempty_visible(io, spec.first_prompt) else {
                return InteractiveOutcome::Done(Outcome::Failure);
            };
            let Ok(secret) = prompt_nonempty_secret(io, second_prompt) else {
                return InteractiveOutcome::Done(Outcome::Failure);
            };
            (first, secret)
        } else {
            // Single-credential source: read the one (secret) value hidden.
            let Ok(secret) = prompt_nonempty_secret(io, spec.first_prompt) else {
                return InteractiveOutcome::Done(Outcome::Failure);
            };
            (String::new(), secret)
        };

        if let Some(confirm) = spec.pre_probe_confirm {
            match read_visible(io, confirm) {
                Ok(ans) if confirm_answer_proceeds(&ans) => {}
                Ok(_) => {
                    println!("Aborted — nothing was saved.");
                    return InteractiveOutcome::Done(Outcome::Failure);
                }
                Err(_) => return InteractiveOutcome::Done(Outcome::Failure),
            }
        }

        println!("Checking your {} credentials…", spec.label);
        match probe(first.clone(), secret.clone()).await {
            Ok(count) => {
                // Write order matters: identifier first, secret last — if the
                // second write fails, only the (public) identifier can be
                // orphaned, never the secret, and the next setup overwrites it.
                let saved = if let Some(second_key) = spec.second_key {
                    let first_secret = SecretString::new(first.into_boxed_str());
                    store
                        .set(spec.first_key, &first_secret)
                        .and_then(|()| store.set(second_key, &secret))
                } else {
                    store.set(spec.first_key, &secret)
                };
                return InteractiveOutcome::Done(match saved {
                    Ok(()) => {
                        println!("{}", verify_ok_text(spec.label, count, spec.try_cmd));
                        println!(
                            "   Saved to your OS keychain — you're set. (Env vars still override.)"
                        );
                        Outcome::Success
                    }
                    Err(e) => {
                        println!("{}", verify_ok_text(spec.label, count, spec.try_cmd));
                        let export_lines = [Some(spec.first_key), spec.second_key]
                            .into_iter()
                            .flatten()
                            .map(|key| format!("export {key}=paste_your_value"))
                            .collect::<Vec<_>>()
                            .join("\n   ");
                        println!(
                            "⚠  Verified, but saving to the OS keychain failed: {e}\n   \
                             Fall back to environment variables (with your real values):\n\n   \
                             {export_lines}"
                        );
                        Outcome::Failure
                    }
                });
            }
            Err(e) => {
                println!("{}", verify_err_text(&e, spec.unauthorized_hint));
                if attempt < MAX_ATTEMPTS {
                    println!("Let's try again ({} of {MAX_ATTEMPTS}).", attempt + 1);
                }
            }
        }
    }
    println!(
        "Still not verified after {MAX_ATTEMPTS} attempts — double-check the values and re-run `openintel setup`."
    );
    InteractiveOutcome::Done(Outcome::Failure)
}

/// Empty-input re-asks are bounded so a broken/EOF'd stream (or an empty
/// string forever, e.g. rpassword at EOF) can't spin the loop unbounded.
const MAX_EMPTY_REASKS: usize = 3;

/// Visible prompt (identifier-like values). Re-asks on empty input.
fn prompt_nonempty_visible(io: &mut SetupIo<'_>, label: &str) -> std::io::Result<String> {
    for _ in 0..MAX_EMPTY_REASKS {
        let ans = read_visible(io, &format!("{label}: "))?;
        let trimmed = ans.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
        println!("Please enter a value.");
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::UnexpectedEof,
        "no input provided",
    ))
}

/// Hidden prompt (secret-like values). Re-asks on empty input.
fn prompt_nonempty_secret(io: &mut SetupIo<'_>, label: &str) -> std::io::Result<SecretString> {
    use secrecy::ExposeSecret as _;
    for _ in 0..MAX_EMPTY_REASKS {
        let secret = (io.read_secret)(&format!("{label} (input hidden): "))?;
        if !secret.expose_secret().trim().is_empty() {
            return Ok(secret);
        }
        println!("Please enter a value.");
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::UnexpectedEof,
        "no input provided",
    ))
}

fn read_visible(io: &mut SetupIo<'_>, prompt: &str) -> std::io::Result<String> {
    print!("{prompt}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    if io.input.read_line(&mut line)? == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "input closed",
        ));
    }
    Ok(line)
}

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

/// One paid round trip: search the default macro accounts for AAPL (max 10 reads ≈ $0.05).
async fn probe_x(bearer: SecretString) -> Result<usize, DomainError> {
    let feed = XPulseSource::new(bearer)?;
    let ticker = Ticker::parse("AAPL")?;
    let accounts: Vec<String> = crate::application::pulse::DEFAULT_PULSE_ACCOUNTS
        .iter()
        .map(|s| s.to_string())
        .collect();
    let fetch = feed.pulse(&ticker, &accounts, &[], 24, 10).await?;
    Ok(fetch.posts.len()) // display count for the "pulled N posts" message, not billing count
}

async fn setup_x(credentials: &Credentials) -> ExitCode {
    match credentials.x_bearer.clone() {
        None => {
            println!("{}", X_SPEC.condensed_guide);
            println!(
                "Set OPENINTEL_X_BEARER (or run `openintel setup x` in a terminal), then re-run."
            );
            ExitCode::FAILURE
        }
        Some(bearer) => {
            println!("Checking your X credentials… (reads up to 10 posts ≈ $0.05)");
            match probe_x(bearer).await {
                Ok(count) => {
                    println!("{}", verify_ok_text("X", count, X_SPEC.try_cmd));
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    println!("{}", verify_err_text(&e, X_UNAUTHORIZED_HINT));
                    ExitCode::FAILURE
                }
            }
        }
    }
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
    fn verify_err_text_maps_forbidden_to_credits_hint() {
        let forbidden = DomainError::SourceFailure {
            name: "x".into(),
            message: "forbidden — check API access and credit balance".into(),
        };
        let text = verify_err_text(&forbidden, X_UNAUTHORIZED_HINT);
        assert!(text.contains("credits"));
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

    use crate::config::store::InMemoryStore;
    use secrecy::ExposeSecret;
    use std::io::Cursor;

    fn scripted<'a>(
        input: &'a mut Cursor<&'static str>,
        secrets: &'a dyn Fn(&str) -> std::io::Result<SecretString>,
    ) -> SetupIo<'a> {
        SetupIo {
            input,
            read_secret: secrets,
        }
    }

    fn ok_secret(_prompt: &str) -> std::io::Result<SecretString> {
        Ok(SecretString::new("s3cret".to_string().into_boxed_str()))
    }

    #[tokio::test]
    async fn interactive_happy_path_saves_both_keys() {
        let store = InMemoryStore::new();
        let mut input = Cursor::new("my-id\n");
        let mut io = scripted(&mut input, &ok_secret);
        let outcome = run_interactive(&mut io, &store, &REDDIT_SPEC, None, |_first, _secret| {
            std::future::ready(Ok(2usize))
        })
        .await;
        assert!(matches!(
            outcome,
            InteractiveOutcome::Done(Outcome::Success)
        ));
        let map = store.map.borrow();
        assert_eq!(
            map.get("OPENINTEL_REDDIT_CLIENT_ID")
                .unwrap()
                .expose_secret(),
            "my-id"
        );
        assert_eq!(
            map.get("OPENINTEL_REDDIT_CLIENT_SECRET")
                .unwrap()
                .expose_secret(),
            "s3cret"
        );
    }

    #[tokio::test]
    async fn interactive_three_failures_exits_one_and_saves_nothing() {
        let store = InMemoryStore::new();
        let mut input = Cursor::new("id1\nid2\nid3\n");
        let mut io = scripted(&mut input, &ok_secret);
        let outcome = run_interactive(&mut io, &store, &REDDIT_SPEC, None, |_f, _s| {
            std::future::ready(Err(DomainError::SourceFailure {
                name: "reddit".into(),
                message: "unauthorized — check client id/secret".into(),
            }))
        })
        .await;
        assert!(matches!(
            outcome,
            InteractiveOutcome::Done(Outcome::Failure)
        ));
        assert!(store.map.borrow().is_empty());
    }

    #[tokio::test]
    async fn interactive_empty_input_reasks_then_succeeds() {
        let store = InMemoryStore::new();
        // First visible answer empty -> re-ask -> then a real id.
        let mut input = Cursor::new("\nreal-id\n");
        let mut io = scripted(&mut input, &ok_secret);
        let outcome = run_interactive(&mut io, &store, &REDDIT_SPEC, None, |first, _s| {
            std::future::ready(if first == "real-id" {
                Ok(0)
            } else {
                Err(DomainError::NoData)
            })
        })
        .await;
        assert!(matches!(
            outcome,
            InteractiveOutcome::Done(Outcome::Success)
        ));
    }

    #[tokio::test]
    async fn interactive_replace_declined_verifies_existing() {
        let store = InMemoryStore::new();
        let mut input = Cursor::new("n\n");
        let mut io = scripted(&mut input, &ok_secret);
        let outcome = run_interactive(
            &mut io,
            &store,
            &REDDIT_SPEC,
            Some("the OS keychain"),
            |_f, _s| std::future::ready(Ok(1)),
        )
        .await;
        assert!(matches!(outcome, InteractiveOutcome::VerifyExisting));
        assert!(store.map.borrow().is_empty());
    }

    #[tokio::test]
    async fn interactive_replace_blank_enter_defaults_to_decline() {
        let store = InMemoryStore::new();
        let mut input = Cursor::new("\n");
        let mut io = scripted(&mut input, &ok_secret);
        let outcome = run_interactive(
            &mut io,
            &store,
            &REDDIT_SPEC,
            Some("the OS keychain"),
            |_f, _s| std::future::ready(Ok(1)),
        )
        .await;
        assert!(matches!(outcome, InteractiveOutcome::VerifyExisting));
        assert!(store.map.borrow().is_empty());
    }

    #[tokio::test]
    async fn interactive_eof_at_prompt_fails_cleanly() {
        let store = InMemoryStore::new();
        let mut input = Cursor::new(""); // immediate EOF
        let mut io = scripted(&mut input, &ok_secret);
        let outcome = run_interactive(&mut io, &store, &REDDIT_SPEC, None, |_f, _s| {
            std::future::ready(Ok(1usize))
        })
        .await;
        assert!(matches!(
            outcome,
            InteractiveOutcome::Done(Outcome::Failure)
        ));
        assert!(store.map.borrow().is_empty());
    }

    #[tokio::test]
    async fn interactive_persistent_empty_secret_fails_cleanly() {
        fn empty_secret(_prompt: &str) -> std::io::Result<SecretString> {
            Ok(SecretString::new("".to_string().into_boxed_str()))
        }
        let store = InMemoryStore::new();
        let mut input = Cursor::new("my-id\n");
        let mut io = scripted(&mut input, &empty_secret);
        let outcome = run_interactive(&mut io, &store, &REDDIT_SPEC, None, |_f, _s| {
            std::future::ready(Ok(1usize))
        })
        .await;
        assert!(matches!(
            outcome,
            InteractiveOutcome::Done(Outcome::Failure)
        ));
        assert!(store.map.borrow().is_empty());
    }

    #[tokio::test]
    async fn interactive_persistent_blank_visible_input_fails_cleanly() {
        let store = InMemoryStore::new();
        let mut input = Cursor::new("\n\n\n\n"); // blank lines, no EOF within the bound
        let mut io = scripted(&mut input, &ok_secret);
        let outcome = run_interactive(&mut io, &store, &REDDIT_SPEC, None, |_f, _s| {
            std::future::ready(Ok(1usize))
        })
        .await;
        assert!(matches!(
            outcome,
            InteractiveOutcome::Done(Outcome::Failure)
        ));
        assert!(store.map.borrow().is_empty());
    }

    #[tokio::test]
    async fn interactive_save_failure_is_exit_one_with_fallback() {
        let store = InMemoryStore::failing();
        let mut input = Cursor::new("my-id\n");
        let mut io = scripted(&mut input, &ok_secret);
        let outcome = run_interactive(&mut io, &store, &REDDIT_SPEC, None, |_f, _s| {
            std::future::ready(Ok(1usize))
        })
        .await;
        assert!(matches!(
            outcome,
            InteractiveOutcome::Done(Outcome::Failure)
        ));
    }

    #[test]
    fn forget_reports_failure_when_store_is_broken() {
        let store = InMemoryStore::failing();
        assert_eq!(forget_outcome(&REDDIT_SPEC, &store), Outcome::Failure);
    }

    #[test]
    fn forget_is_idempotent_and_removes_keys() {
        let store = InMemoryStore::new()
            .seed("OPENINTEL_REDDIT_CLIENT_ID", "x")
            .seed("OPENINTEL_REDDIT_CLIENT_SECRET", "y");
        assert_eq!(forget_outcome(&REDDIT_SPEC, &store), Outcome::Success);
        assert!(store.map.borrow().is_empty());
        // Second run: nothing left to delete, still success.
        assert_eq!(forget_outcome(&REDDIT_SPEC, &store), Outcome::Success);
    }

    #[test]
    fn condensed_guides_have_no_export_lines() {
        for spec in [&REDDIT_SPEC, &BLUESKY_SPEC] {
            assert!(!spec.condensed_guide.contains("export "));
            assert!(spec.condensed_guide.contains("keychain"));
        }
        assert!(REDDIT_SPEC.condensed_guide.contains("prefs/apps"));
        assert!(BLUESKY_SPEC.condensed_guide.contains("app-passwords"));
    }

    #[test]
    fn x_guide_contains_cost_and_console_steps() {
        assert!(X_SPEC.condensed_guide.contains("developer.x.com"));
        assert!(X_SPEC.condensed_guide.contains("$0.005"));
        assert!(X_SPEC.condensed_guide.contains("Bearer Token"));
        assert!(X_SPEC.second_key.is_none());
    }

    #[tokio::test]
    async fn single_cred_happy_path_saves_one_key() {
        let store = InMemoryStore::new();
        let mut input = Cursor::new("\n"); // cost-confirm: blank = Yes
        let mut io = scripted(&mut input, &ok_secret);
        let outcome = run_interactive(&mut io, &store, &X_SPEC, None, |_f, _s| {
            std::future::ready(Ok(2usize))
        })
        .await;
        assert!(matches!(
            outcome,
            InteractiveOutcome::Done(Outcome::Success)
        ));
        let map = store.map.borrow();
        assert_eq!(map.len(), 1);
        assert_eq!(
            map.get("OPENINTEL_X_BEARER").unwrap().expose_secret(),
            "s3cret"
        );
    }

    #[tokio::test]
    async fn single_cred_cost_confirm_decline_saves_nothing() {
        let store = InMemoryStore::new();
        let mut input = Cursor::new("n\n");
        let mut io = scripted(&mut input, &ok_secret);
        let outcome = run_interactive(&mut io, &store, &X_SPEC, None, |_f, _s| {
            std::future::ready(Ok(2usize))
        })
        .await;
        assert!(matches!(
            outcome,
            InteractiveOutcome::Done(Outcome::Failure)
        ));
        assert!(store.map.borrow().is_empty());
    }

    #[test]
    fn confirm_answer_proceeds_accepts_blank_and_yes_variants() {
        assert!(confirm_answer_proceeds(""));
        assert!(confirm_answer_proceeds("y"));
        assert!(confirm_answer_proceeds("Y"));
        assert!(confirm_answer_proceeds("yes"));
        assert!(confirm_answer_proceeds("YES"));
        assert!(!confirm_answer_proceeds("n"));
        assert!(!confirm_answer_proceeds("no"));
        assert!(!confirm_answer_proceeds("x"));
    }

    #[test]
    fn forget_x_removes_single_key() {
        let store = InMemoryStore::new().seed("OPENINTEL_X_BEARER", "tok");
        assert_eq!(forget_outcome(&X_SPEC, &store), Outcome::Success);
        assert!(store.map.borrow().is_empty());
    }
}
