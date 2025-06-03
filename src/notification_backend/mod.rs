pub mod in_memory;

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

/// A single notification message, currently just a string payload.
/// We can extend this struct to include metadata (timestamp, sender, etc.).
#[derive(Debug, Clone)]
pub struct NotificationMessage {
    pub id: u64,
    pub timestamp: i64,
    pub payload: String,
}

/// This trait defines the interface for any notification notification_backend.
/// It allows sending a message and subscribing to new messages on a topic.
/// 'async_trait' is used so trait methods can be async.
#[async_trait]
pub trait NotificationBackend: Send + Sync {
    async fn put_messages(&self, topic: &str, payload: String) -> Result<()>;
    async fn get_messages(
        &self,
        topic: &str,
        since_id: Option<u64>,
        since_timestamp: Option<i64>,
        limit: usize,
    ) -> Result<Vec<NotificationMessage>>;
}

pub async fn build_backend(
    config: &crate::configuration::NotificationBackendSettings,
) -> Result<Arc<dyn NotificationBackend>> {
    match config.kind.as_str() {
        "in_memory" => Ok(Arc::new(in_memory::InMemoryBackend::new())),
        kind => Err(anyhow::anyhow!("Unknown notification_backend kind: {kind}")),
    }
}
