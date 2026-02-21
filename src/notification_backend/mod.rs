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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteMessageResult {
    Deleted,
    NotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackendCapabilities {
    pub retention_time: bool,
    pub max_messages: bool,
    pub max_size: bool,
    pub allow_duplicates: bool,
    pub compression: bool,
}

pub const JETSTREAM_CAPABILITIES: BackendCapabilities = BackendCapabilities {
    retention_time: true,
    max_messages: true,
    max_size: true,
    allow_duplicates: true,
    compression: true,
};

pub const IN_MEMORY_CAPABILITIES: BackendCapabilities = BackendCapabilities {
    // In-memory backend intentionally rejects schema storage_policy fields.
    // Capacity/eviction behavior is controlled only by backend-level in_memory settings.
    retention_time: false,
    max_messages: false,
    max_size: false,
    allow_duplicates: false,
    compression: false,
};

pub fn capabilities_for_backend_kind(kind: &str) -> Option<BackendCapabilities> {
    match kind {
        "jetstream" => Some(JETSTREAM_CAPABILITIES),
        "in_memory" => Some(IN_MEMORY_CAPABILITIES),
        _ => None,
    }
}

/// Trait defining the interface for notification backends
///
/// This abstraction allows different storage backends (in-memory, JetStream etc.)
/// to be used interchangeably while maintaining the same interface.
#[async_trait]
pub trait NotificationBackend: Send + Sync {
    fn capabilities(&self) -> BackendCapabilities;
    async fn put_messages(&self, topic: &str, payload: String) -> Result<()>;
    async fn put_message_with_headers(
        &self,
        topic: &str,
        headers: Option<HashMap<String, String>>,
        payload: String,
    ) -> Result<()>;
    async fn wipe_stream(&self, stream_name: &str) -> Result<()>;
    async fn wipe_all(&self) -> Result<()>;
    async fn delete_message(&self, stream_key: &str, sequence: u64) -> Result<DeleteMessageResult>;
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
            let cfg = JetStreamConfig::from_backend_settings(config)?;
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

#[cfg(test)]
mod tests {
    use super::capabilities_for_backend_kind;

    #[test]
    fn capability_map_for_known_backends_is_stable() {
        let jetstream = capabilities_for_backend_kind("jetstream").expect("jetstream exists");
        assert!(jetstream.retention_time);
        assert!(jetstream.max_messages);
        assert!(jetstream.max_size);
        assert!(jetstream.allow_duplicates);
        assert!(jetstream.compression);

        let in_memory = capabilities_for_backend_kind("in_memory").expect("in_memory exists");
        assert!(!in_memory.retention_time);
        assert!(!in_memory.max_messages);
        assert!(!in_memory.max_size);
        assert!(!in_memory.allow_duplicates);
        assert!(!in_memory.compression);
    }
}
