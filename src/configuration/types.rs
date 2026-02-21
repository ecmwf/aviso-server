use aviso_validators::ValidationRules;
use serde::{Deserialize, Serialize};
use serde_aux::field_attributes::deserialize_number_from_string;
use std::collections::HashMap;
use utoipa::ToSchema;

/// Configuration for watch/replay streaming behavior.
///
/// These defaults are operationally meaningful; increasing replay-related fields
/// improves catch-up throughput but can increase memory/CPU pressure.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct WatchEndpointSettings {
    pub sse_heartbeat_interval_sec: u64,
    pub connection_max_duration_sec: u64,
    pub replay_batch_size: usize,
    pub max_historical_notifications: usize,
    pub replay_batch_delay_ms: u64,
    pub concurrent_notification_processing: usize,
}

impl Default for WatchEndpointSettings {
    fn default() -> Self {
        Self {
            sse_heartbeat_interval_sec: 30,
            connection_max_duration_sec: 3600,
            replay_batch_size: 100,
            max_historical_notifications: 10000,
            replay_batch_delay_ms: 100,
            concurrent_notification_processing: 15,
        }
    }
}

#[derive(serde::Deserialize, Serialize, Clone, Debug, ToSchema)]
pub struct PayloadConfig {
    #[serde(rename = "type")]
    pub allowed_types: Vec<String>,
    #[schema(example = true)]
    pub required: bool,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct TopicConfig {
    pub base: String,
    pub key_order: Vec<String>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct EventSchema {
    pub payload: Option<PayloadConfig>,
    pub topic: Option<TopicConfig>,
    pub endpoint: Option<TopicConfig>,
    pub identifier: HashMap<String, Vec<ValidationRules>>,
}

/// Client-facing schema shape for schema endpoints.
#[derive(Deserialize, Serialize, Clone, Debug, ToSchema)]
pub struct ApiEventSchema {
    pub payload: Option<PayloadConfig>,
    pub identifier: HashMap<String, Vec<ValidationRules>>,
}

impl From<&EventSchema> for ApiEventSchema {
    fn from(schema: &EventSchema) -> Self {
        Self {
            payload: schema.payload.clone(),
            identifier: schema.identifier.clone(),
        }
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct LoggingSettings {
    pub level: String,
    /// Compatibility field; runtime output is OTel-style JSON regardless of value.
    pub format: String,
}

#[derive(serde::Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "lowercase")]
pub enum JetStreamStorageType {
    File,
    Memory,
}

#[derive(serde::Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "lowercase")]
pub enum JetStreamRetentionPolicy {
    Limits,
    Interest,
    Workqueue,
}

#[derive(serde::Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "lowercase")]
pub enum JetStreamDiscardPolicy {
    Old,
    New,
}

#[derive(serde::Deserialize, Serialize, Clone, Debug)]
pub struct JetStreamSettings {
    pub nats_url: Option<String>,
    pub token: Option<String>,
    pub timeout_seconds: Option<u64>,
    pub retry_attempts: Option<u32>,
    pub max_messages: Option<i64>,
    pub max_bytes: Option<i64>,
    pub retention_days: Option<u32>,
    pub storage_type: Option<JetStreamStorageType>,
    pub replicas: Option<usize>,
    pub retention_policy: Option<JetStreamRetentionPolicy>,
    pub discard_policy: Option<JetStreamDiscardPolicy>,
    pub enable_auto_reconnect: Option<bool>,
    pub max_reconnect_attempts: Option<u32>,
    pub reconnect_delay_ms: Option<u64>,
}

#[derive(serde::Deserialize, Serialize, Clone, Debug)]
pub struct InMemorySettings {
    pub max_history_per_topic: Option<usize>,
    pub max_topics: Option<usize>,
    pub enable_metrics: Option<bool>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct NotificationBackendSettings {
    pub kind: String,
    #[serde(default)]
    pub in_memory: Option<InMemorySettings>,
    #[serde(default)]
    pub jetstream: Option<JetStreamSettings>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct ApplicationSettings {
    pub host: String,
    #[serde(deserialize_with = "deserialize_number_from_string")]
    pub port: u16,
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default = "default_static_files_path")]
    pub static_files_path: String,
}

fn default_base_url() -> String {
    "http://localhost".to_string()
}

fn default_static_files_path() -> String {
    "/app/static".to_string()
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Settings {
    /// HTTP/server process settings.
    pub application: ApplicationSettings,
    /// Backend selection and backend-specific defaults.
    pub notification_backend: NotificationBackendSettings,
    pub logging: Option<LoggingSettings>,
    /// Event schema definitions keyed by event type.
    pub notification_schema: Option<HashMap<String, EventSchema>>,
    #[serde(default)]
    pub watch_endpoint: WatchEndpointSettings,
}

#[cfg(test)]
mod tests {
    use super::{JetStreamSettings, JetStreamStorageType};

    #[test]
    fn jetstream_settings_accept_lowercase_storage_type() {
        let settings: JetStreamSettings =
            serde_json::from_str(r#"{"storage_type":"file"}"#).expect("should deserialize");
        assert!(matches!(
            settings.storage_type,
            Some(JetStreamStorageType::File)
        ));
    }

    #[test]
    fn jetstream_settings_reject_invalid_storage_type() {
        let result = serde_json::from_str::<JetStreamSettings>(r#"{"storage_type":"disk"}"#);
        assert!(result.is_err());
    }
}
