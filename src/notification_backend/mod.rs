pub mod in_memory;
pub mod jetstream;
pub mod replay;

pub use jetstream::backend::JetStreamBackend;
pub use jetstream::config::JetStreamConfig;
use std::collections::HashMap;

use crate::telemetry::{SERVICE_NAME, SERVICE_VERSION};
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures_util::Stream;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Notification message structure for backend storage
///
/// This represents a single notification message with metadata.
/// The ID is typically assigned by the backend (e.g., JetStream sequence number).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationMessage {
    /// Backend-specific sequence number (JetStream sequence, InMemory counter)
    pub sequence: u64,
    /// Full topic string for the message
    pub topic: String,
    /// Message payload as stored in backend
    pub payload: String,
    /// Message timestamp from backend
    pub timestamp: Option<DateTime<Utc>>,
    /// Optional metadata
    pub metadata: Option<HashMap<String, String>>,
}

/// Trait defining the interface for notification backends
///
/// This abstraction allows different storage backends (in-memory, JetStream etc.)
/// to be used interchangeably while maintaining the same interface.
#[async_trait]
pub trait NotificationBackend: Send + Sync {
    async fn put_messages(&self, topic: &str, payload: String) -> Result<()>;
    async fn put_message_with_headers(
        &self,
        topic: &str,
        headers: Option<HashMap<String, String>>,
        payload: String,
    ) -> Result<()>;
    async fn wipe_stream(&self, stream_name: &str) -> Result<()>;
    async fn wipe_all(&self) -> Result<()>;
    async fn get_messages_batch(
        &self,
        params: replay::BatchParams,
    ) -> Result<crate::types::BatchResult>;
    async fn subscribe_to_topic(
        &self,
        topic: &str,
    ) -> Result<Box<dyn Stream<Item = NotificationMessage> + Unpin + Send>>;

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
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
            tracing::info!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_domain = "backend",
                event_name = "backend.in_memory.initialization.started",
                "Building in-memory notification backend"
            );
            let cfg = in_memory::InMemoryConfig::from_backend_settings(config);
            Ok(Arc::new(in_memory::InMemoryBackend::new(cfg)))
        }
        "jetstream" => {
            tracing::info!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_domain = "backend",
                event_name = "backend.jetstream.initialization.started",
                "Building JetStream notification backend"
            );
            let cfg = JetStreamConfig::from_backend_settings(config);
            cfg.validate()?;
            if cfg.token.is_some() {
                tracing::info!(
                    service_name = SERVICE_NAME,
                    service_version = SERVICE_VERSION,
                    event_domain = "backend",
                    event_name = "backend.jetstream.auth.token_configured",
                    "NATS token configured"
                );
            } else {
                tracing::info!(
                    service_name = SERVICE_NAME,
                    service_version = SERVICE_VERSION,
                    event_domain = "backend",
                    event_name = "backend.jetstream.auth.unauthenticated",
                    "No NATS token configured - using unauthenticated connection"
                );
            }
            Ok(Arc::new(JetStreamBackend::new(cfg).await?))
        }
        kind => Err(anyhow::anyhow!("Unknown notification_backend kind: {kind}")),
    }
}
