pub mod mock_bluesky;
pub mod mock_reddit;
pub mod mock_x;
pub mod reddit;

use crate::config::secrets::Credentials;
use crate::domain::ports::social_data_source::SocialDataSource;

/// Assemble the social data sources from credentials: the real `RedditSource`
/// when both OAuth credentials are set (a `RedditSource::new` failure logs a
/// warning to stderr and omits it), plus the mock X and Bluesky sources.
/// Shared by both composition roots (`main.rs` and `mcp::server::serve`).
pub fn build_social_sources(credentials: &Credentials) -> Vec<Box<dyn SocialDataSource>> {
    let mut social: Vec<Box<dyn SocialDataSource>> = Vec::new();
    if let (Some(id), Some(secret)) = (
        credentials.reddit_client_id.clone(),
        credentials.reddit_client_secret.clone(),
    ) {
        match reddit::RedditSource::new(id, secret) {
            Ok(src) => social.push(Box::new(src)),
            Err(e) => eprintln!("warning: reddit disabled: {e}"),
        }
    }
    social.push(Box::new(mock_x::MockXSource));
    social.push(Box::new(mock_bluesky::MockBlueskySource));
    social
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::values::source_kind::SourceKind;
    use secrecy::SecretString;

    fn creds(reddit: bool) -> Credentials {
        let s = |v: &str| Some(SecretString::new(v.to_string().into_boxed_str()));
        Credentials {
            reddit_client_id: if reddit { s("id") } else { None },
            reddit_client_secret: if reddit { s("secret") } else { None },
            x_bearer: None,
            bluesky_app_password: None,
            market_api_key: None,
        }
    }

    #[test]
    fn omits_reddit_without_creds() {
        let kinds: Vec<_> = build_social_sources(&creds(false))
            .iter()
            .map(|s| s.kind())
            .collect();
        assert_eq!(kinds, vec![SourceKind::X, SourceKind::Bluesky]);
    }

    #[test]
    fn includes_reddit_with_creds() {
        let kinds: Vec<_> = build_social_sources(&creds(true))
            .iter()
            .map(|s| s.kind())
            .collect();
        assert_eq!(
            kinds,
            vec![SourceKind::Reddit, SourceKind::X, SourceKind::Bluesky]
        );
    }
}
