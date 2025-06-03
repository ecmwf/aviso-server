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
    /// Remove all notifications in a specific stream
    /// For JetStream: purges the entire stream (e.g., "DISS", "MARS")
    /// For in-memory: removes all subjects matching the stream pattern
    async fn wipe_stream(&self, stream_name: &str) -> Result<()>;

    /// Remove all data from all streams
    /// This is a complete reset of the backend storage
    async fn wipe_all(&self) -> Result<()>;
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
            let in_memory_config = in_memory::InMemoryConfig::from_backend_settings(config);
            Ok(Arc::new(in_memory::InMemoryBackend::new(in_memory_config)))
        }
        "jetstream" => {
            tracing::info!("Building JetStream notification backend");
            let jetstream_config = jetstream::JetStreamConfig::from_backend_settings(config);

            if jetstream_config.token.is_some() {
                tracing::info!("NATS token configured");
            } else {
                tracing::info!("No NATS token configured - using unauthenticated connection");
            }

            let backend = jetstream::JetStreamBackend::new(jetstream_config).await?;
            Ok(Arc::new(backend))
        }
        kind => Err(anyhow::anyhow!("Unknown notification_backend kind: {kind}")),
    }
}
