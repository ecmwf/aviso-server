pub mod in_memory;
pub mod jetstream;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Notification message structure for backend storage
///
/// This represents a single notification message with metadata.
/// The ID is typically assigned by the backend (e.g., JetStream sequence number).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationMessage {
    pub id: u64,
    pub timestamp: i64,
    pub payload: String,
}

/// Trait defining the interface for notification backends
///
/// This abstraction allows different storage backends (in-memory, JetStream etc.)
/// to be used interchangeably while maintaining the same interface.
#[async_trait]
pub trait NotificationBackend: Send + Sync {
    /// Store a notification message for a given topic
    ///
    /// The topic is built by your TopicBuilder and will be used directly
    /// as the storage key/subject. For JetStream, this becomes the NATS subject.
    async fn put_messages(&self, topic: &str, payload: String) -> Result<()>;
}

/// Build the appropriate notification backend based on configuration
///
/// This factory function creates the backend instance and handles any initialization
/// required. For JetStream, this includes connecting to NATS and ensuring the stream exists.
pub async fn build_backend(
    config: &crate::configuration::NotificationBackendSettings,
) -> Result<Arc<dyn NotificationBackend>> {
    match config.kind.as_str() {
        "in_memory" => {
            tracing::info!("Building in-memory notification backend");
            Ok(Arc::new(in_memory::InMemoryBackend::new()))
        }
        "jetstream" => {
            tracing::info!("Building JetStream notification backend");
            let jetstream_config = jetstream::JetStreamConfig::from_backend_settings(config);

            // Log token source for debugging (without revealing the token)
            if jetstream_config.token.is_some() {
                let token_source = if config
                    .jetstream
                    .as_ref()
                    .and_then(|js| js.token.as_ref())
                    .is_some()
                {
                    "config.yaml"
                } else {
                    "environment variable"
                };
                tracing::info!(token_source = token_source, "NATS token configured");
            } else {
                tracing::info!("No NATS token configured - using unauthenticated connection");
            }

            let backend = jetstream::JetStreamBackend::new(jetstream_config).await?;
            Ok(Arc::new(backend))
        }
        kind => Err(anyhow::anyhow!("Unknown notification_backend kind: {kind}")),
    }
}
