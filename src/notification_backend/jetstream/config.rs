use crate::configuration::NotificationBackendSettings;

/// Configuration for JetStream backend
/// Contains all necessary settings for connecting to NATS and configuring streams
#[derive(Debug, Clone)]
pub struct JetStreamConfig {
    /// NATS server URL (e.g., "nats://localhost:4222")
    pub nats_url: String,
    /// Connection timeout in seconds
    pub timeout_seconds: u64,
    /// Number of retry attempts for failed operations
    pub retry_attempts: u32,
    /// Optional authentication token for NATS
    pub token: Option<String>,
    /// Maximum number of messages per stream
    pub max_messages: Option<i64>,
    /// Maximum bytes per stream
    pub max_bytes: Option<i64>,
    /// Maximum age of messages in seconds
    pub retention_days: Option<u32>,
    /// Storage type: "file" or "memory"
    pub storage_type: String,
    /// Number of replicas for high availability
    pub replicas: Option<usize>,
    /// Retention policy: "limits", "interest", or "workqueue"
    pub retention_policy: String,
    /// Discard policy when limits are reached: "old" or "new"
    pub discard_policy: String,
    /// Enable automatic reconnection on failures
    pub enable_auto_reconnect: bool,
    /// Maximum reconnection attempts before giving up temporarily
    pub max_reconnect_attempts: u32,
    /// Base delay between reconnection attempts in milliseconds
    pub reconnect_delay_ms: u64,
}

impl JetStreamConfig {
    /// Create JetStreamConfig from application configuration
    /// Merges configuration file settings
    /// Environment variables take precedence over config file values
    pub fn from_backend_settings(settings: &NotificationBackendSettings) -> Self {
        let js_settings = settings.jetstream.as_ref();
        Self {
            nats_url: js_settings
                .and_then(|js| js.nats_url.clone())
                .unwrap_or_else(|| "nats://localhost:4222".to_string()),
            timeout_seconds: js_settings.and_then(|js| js.timeout_seconds).unwrap_or(30),
            retry_attempts: js_settings.and_then(|js| js.retry_attempts).unwrap_or(3),
            token: js_settings
                .and_then(|js| js.token.clone())
                .or_else(|| std::env::var("NATS_TOKEN").ok()),
            max_messages: js_settings.and_then(|js| js.max_messages),
            max_bytes: js_settings.and_then(|js| js.max_bytes),
            retention_days: js_settings.and_then(|js| js.retention_days),
            storage_type: js_settings
                .and_then(|js| js.storage_type.clone())
                .unwrap_or_else(|| "file".to_string()),
            replicas: js_settings.and_then(|js| js.replicas),
            retention_policy: js_settings
                .and_then(|js| js.retention_policy.clone())
                .unwrap_or_else(|| "limits".to_string()),
            discard_policy: js_settings
                .and_then(|js| js.discard_policy.clone())
                .unwrap_or_else(|| "old".to_string()),
            enable_auto_reconnect: js_settings
                .and_then(|js| js.enable_auto_reconnect)
                .unwrap_or(true),
            max_reconnect_attempts: js_settings
                .and_then(|js| js.max_reconnect_attempts)
                .unwrap_or(5),
            reconnect_delay_ms: js_settings
                .and_then(|js| js.reconnect_delay_ms)
                .unwrap_or(2000),
        }
    }
}
