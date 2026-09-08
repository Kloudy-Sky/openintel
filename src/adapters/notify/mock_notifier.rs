use std::sync::Mutex;

use async_trait::async_trait;

use crate::domain::error::DomainError;
use crate::domain::ports::notifier::Notifier;

/// Test double: records every push, or fails every push.
#[derive(Default)]
pub struct MockNotifier {
    pub sent: Mutex<Vec<(String, String)>>,
    pub fail_with: Option<String>,
}

#[async_trait]
impl Notifier for MockNotifier {
    async fn notify(&self, title: &str, body: &str) -> Result<(), DomainError> {
        if let Some(message) = &self.fail_with {
            return Err(DomainError::SourceFailure {
                name: "mock-notifier".into(),
                message: message.clone(),
            });
        }
        self.sent
            .lock()
            .expect("mock notifier lock")
            .push((title.to_string(), body.to_string()));
        Ok(())
    }
}
