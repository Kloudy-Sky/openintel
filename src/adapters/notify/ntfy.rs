//! ntfy.sh push: a plain POST with a Title header. The topic name is the only
//! secret, so it is held as a SecretString and never logged.

use std::time::Duration;

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};

use crate::domain::error::DomainError;
use crate::domain::ports::notifier::Notifier;

const BASE_URL: &str = "https://ntfy.sh";
const TIMEOUT_SECS: u64 = 20;

fn fail(message: impl Into<String>) -> DomainError {
    DomainError::SourceFailure {
        name: "ntfy".into(),
        message: message.into(),
    }
}

pub struct NtfyNotifier {
    client: reqwest::Client,
    topic: SecretString,
}

impl NtfyNotifier {
    pub fn new(topic: SecretString) -> Result<Self, DomainError> {
        if topic.expose_secret().trim().is_empty() {
            return Err(fail("ntfy topic is empty"));
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(TIMEOUT_SECS))
            .user_agent(concat!("openintel/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| fail(format!("client build failed: {e}")))?;
        Ok(Self { client, topic })
    }
}

#[async_trait]
impl Notifier for NtfyNotifier {
    async fn notify(&self, title: &str, body: &str) -> Result<(), DomainError> {
        let url = format!("{BASE_URL}/{}", self.topic.expose_secret().trim());
        let resp = self
            .client
            .post(url)
            .header("Title", title)
            .body(body.to_string())
            .send()
            .await
            .map_err(|e| fail(format!("request failed: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(fail(format!("HTTP {status}")));
        }
        Ok(())
    }
}
