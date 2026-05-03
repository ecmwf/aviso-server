// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use super::AuthSettings;
use aviso_validators::ValidationRules;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
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
#[serde(deny_unknown_fields)]
pub struct PayloadConfig {
    /// Whether clients must provide a payload for this event type.
    /// Payload values are always treated as JSON.
    #[schema(example = true)]
    pub required: bool,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct TopicConfig {
    pub base: String,
    pub key_order: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct IdentifierFieldConfig {
    /// Optional human-readable explanation exposed by schema endpoints.
    pub description: Option<String>,
    pub rule: ValidationRules,
}

impl IdentifierFieldConfig {
    pub fn with_rule(rule: ValidationRules) -> Self {
        Self {
            description: None,
            rule,
        }
    }

    pub fn with_description(description: impl Into<String>, rule: ValidationRules) -> Self {
        Self {
            description: Some(description.into()),
            rule,
        }
    }

    pub fn is_required(&self) -> bool {
        self.rule.is_required()
    }
}

#[derive(Deserialize, Serialize)]
struct IdentifierFieldConfigRepr {
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(flatten)]
    rule_fields: HashMap<String, serde_json::Value>,
}

impl<'de> Deserialize<'de> for IdentifierFieldConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Keep config/API shape flat (`description` + rule fields) while
        // reusing `ValidationRules` as the internal representation.
        let repr = IdentifierFieldConfigRepr::deserialize(deserializer)?;
        let rule_value = serde_json::Value::Object(repr.rule_fields.into_iter().collect());
        let rule = serde_json::from_value(rule_value.clone()).map_err(serde::de::Error::custom)?;

        // ValidationRules ignores unknown fields by default, so explicitly reject
        // extras to keep config errors deterministic for operators.
        let normalized_rule = serde_json::to_value(&rule).map_err(serde::de::Error::custom)?;
        let input_fields = rule_value
            .as_object()
            .expect("rule_value is always serialized as object");
        let normalized_fields = normalized_rule
            .as_object()
            .expect("validation rule must serialize to object");
        let unknown_fields: Vec<&str> = input_fields
            .keys()
            .filter(|field| !normalized_fields.contains_key(*field))
            .map(String::as_str)
            .collect();
        if !unknown_fields.is_empty() {
            return Err(serde::de::Error::custom(format!(
                "unknown field(s): {}",
                unknown_fields.join(", ")
            )));
        }

        Ok(Self {
            description: repr.description,
            rule,
        })
    }
}

impl Serialize for IdentifierFieldConfig {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let rule_value = serde_json::to_value(&self.rule).map_err(serde::ser::Error::custom)?;
        let serde_json::Value::Object(rule_fields) = rule_value else {
            return Err(serde::ser::Error::custom(
                "validation rule must serialize to an object",
            ));
        };

        IdentifierFieldConfigRepr {
            description: self.description.clone(),
            rule_fields: rule_fields.into_iter().collect(),
        }
        .serialize(serializer)
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct EventSchema {
    pub payload: Option<PayloadConfig>,
    pub topic: Option<TopicConfig>,
    pub endpoint: Option<TopicConfig>,
    pub identifier: HashMap<String, IdentifierFieldConfig>,
    /// Optional per-schema storage policy (backend capability validated at startup).
    pub storage_policy: Option<EventStoragePolicy>,
    pub auth: Option<StreamAuthConfig>,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct EventStoragePolicy {
    /// Duration literal (for example `1h`, `7d`, `1w`).
    pub retention_time: Option<String>,
    pub max_messages: Option<i64>,
    /// Size literal (for example `100Mi`, `1Gi`).
    pub max_size: Option<String>,
    pub allow_duplicates: Option<bool>,
    pub compression: Option<bool>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct StreamAuthConfig {
    pub required: bool,
    pub read_roles: Option<HashMap<String, Vec<String>>>,
    pub write_roles: Option<HashMap<String, Vec<String>>>,
    #[serde(default)]
    pub plugins: Option<Vec<String>>,
}

/// Client-facing schema shape for schema endpoints.
#[derive(Deserialize, Serialize, Clone, Debug, ToSchema)]
pub struct ApiEventSchema {
    pub payload: Option<PayloadConfig>,
    pub identifier: HashMap<String, ApiIdentifierFieldConfig>,
}

/// OpenAPI-facing identifier field shape for schema endpoints.
///
/// This mirrors the flattened runtime JSON contract:
/// `description` plus handler fields at the same level.
#[derive(Deserialize, Serialize, Clone, Debug, ToSchema)]
pub struct ApiIdentifierFieldConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(flatten)]
    pub rule: ValidationRules,
}

impl From<&IdentifierFieldConfig> for ApiIdentifierFieldConfig {
    fn from(field: &IdentifierFieldConfig) -> Self {
        Self {
            description: field.description.clone(),
            rule: field.rule.clone(),
        }
    }
}

impl From<&EventSchema> for ApiEventSchema {
    fn from(schema: &EventSchema) -> Self {
        Self {
            payload: schema.payload.clone(),
            identifier: schema
                .identifier
                .iter()
                .map(|(key, value)| (key.clone(), ApiIdentifierFieldConfig::from(value)))
                .collect(),
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
    /// Default stream retention window (examples: "7d", "12h", "30m").
    pub retention_time: Option<String>,
    pub storage_type: Option<JetStreamStorageType>,
    pub replicas: Option<usize>,
    pub retention_policy: Option<JetStreamRetentionPolicy>,
    pub discard_policy: Option<JetStreamDiscardPolicy>,
    pub enable_auto_reconnect: Option<bool>,
    pub max_reconnect_attempts: Option<u32>,
    pub reconnect_delay_ms: Option<u64>,
    /// Publish retry attempts for transient channel-closed errors.
    pub publish_retry_attempts: Option<u32>,
    /// Base backoff (milliseconds) for publish retries.
    pub publish_retry_base_delay_ms: Option<u64>,
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
pub struct MetricsSettings {
    #[serde(default)]
    pub enabled: bool,
    /// Bind address for the metrics HTTP server. Defaults to `127.0.0.1`
    /// so the endpoint is not publicly exposed.
    #[serde(default = "default_metrics_host")]
    pub host: String,
    /// Port for the internal metrics HTTP server (serves `/metrics`).
    pub port: Option<u16>,
}

fn default_metrics_host() -> String {
    "127.0.0.1".to_string()
}

impl Default for MetricsSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            host: default_metrics_host(),
            port: None,
        }
    }
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
    #[serde(default)]
    pub auth: AuthSettings,
    #[serde(default)]
    pub metrics: MetricsSettings,
    // When ecmwf feature is enabled, deserialize EcpdsConfig
    #[cfg(feature = "ecpds")]
    pub ecpds: Option<aviso_ecpds::config::EcpdsConfig>,
    // When disabled, silently absorb any 'ecpds' YAML key as raw JSON (no error)
    #[cfg(not(feature = "ecpds"))]
    #[serde(default, rename = "ecpds")]
    pub ecpds: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::{
        ApiEventSchema, EventSchema, IdentifierFieldConfig, JetStreamSettings,
        JetStreamStorageType, PayloadConfig, StreamAuthConfig,
    };
    use aviso_validators::ValidationRules;
    use std::collections::HashMap;

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

    #[test]
    fn payload_config_rejects_legacy_type_list() {
        let result = serde_json::from_str::<PayloadConfig>(
            r#"{"required":true,"type":["String","HashMap"]}"#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn identifier_field_config_accepts_optional_description() {
        let field: IdentifierFieldConfig = serde_json::from_str(
            r#"{
                "description":"MARS class identifier",
                "type":"StringHandler",
                "max_length":2,
                "required":true
            }"#,
        )
        .expect("should deserialize identifier field config");

        assert_eq!(field.description.as_deref(), Some("MARS class identifier"));
        assert!(matches!(
            field.rule,
            ValidationRules::StringHandler {
                max_length: Some(2),
                required: true
            }
        ));
    }

    #[test]
    fn identifier_field_config_serializes_flat_without_rule_wrapper() {
        let field =
            IdentifierFieldConfig::with_rule(ValidationRules::TimeHandler { required: false });

        let serialized =
            serde_json::to_value(&field).expect("should serialize identifier field config");
        let object = serialized
            .as_object()
            .expect("identifier field config should serialize as object");

        assert_eq!(
            object.get("type").and_then(|v| v.as_str()),
            Some("TimeHandler")
        );
        assert_eq!(
            object.get("required").and_then(|v| v.as_bool()),
            Some(false)
        );
        assert!(
            object.get("rule").is_none(),
            "serialized field must not include nested `rule` object"
        );
        assert!(
            object.get("description").is_none(),
            "description should be omitted when not configured"
        );
    }

    #[test]
    fn identifier_field_config_rejects_unknown_rule_field() {
        let result = serde_json::from_str::<IdentifierFieldConfig>(
            r#"{
                "type":"StringHandler",
                "max_length":2,
                "required":true,
                "unknown_field":"oops"
            }"#,
        );

        assert!(result.is_err());
    }

    #[test]
    fn event_schema_deserializes_auth_config() {
        let schema: EventSchema = serde_json::from_str(
            r#"{
                "identifier": {
                    "class": {
                        "type": "StringHandler",
                        "required": true
                    }
                },
                "auth": {
                    "required": true,
                    "read_roles": {"internal": ["consumer", "analyst"]},
                    "write_roles": {"internal": ["producer"]}
                }
            }"#,
        )
        .expect("should deserialize event schema with auth config");

        let auth = schema.auth.expect("auth should be configured");
        assert!(auth.required);
        let read = auth.read_roles.expect("read_roles should be set");
        assert_eq!(
            read.get("internal").map(Vec::as_slice),
            Some(["consumer".to_string(), "analyst".to_string()].as_slice())
        );
        let write = auth.write_roles.expect("write_roles should be set");
        assert_eq!(
            write.get("internal").map(Vec::as_slice),
            Some(["producer".to_string()].as_slice())
        );
    }

    #[test]
    fn event_schema_deserializes_auth_without_role_restrictions() {
        let schema: EventSchema = serde_json::from_str(
            r#"{
                "identifier": {
                    "class": {
                        "type": "StringHandler",
                        "required": true
                    }
                },
                "auth": {
                    "required": false
                }
            }"#,
        )
        .expect("should deserialize event schema with unrestricted auth roles");

        let auth = schema.auth.expect("auth should be configured");
        assert!(!auth.required);
        assert_eq!(auth.read_roles, None);
        assert_eq!(auth.write_roles, None);
    }

    #[test]
    fn event_schema_rejects_unknown_stream_auth_fields() {
        let result: Result<EventSchema, _> = serde_json::from_str(
            r#"{
                "identifier": {
                    "class": {
                        "type": "StringHandler",
                        "required": true
                    }
                },
                "auth": {
                    "required": true,
                    "allowed_roles": ["admin"]
                }
            }"#,
        );

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("allowed_roles"));
    }

    #[test]
    fn api_event_schema_does_not_expose_auth_config() {
        let schema = EventSchema {
            payload: Some(PayloadConfig { required: true }),
            topic: None,
            endpoint: None,
            identifier: HashMap::new(),
            storage_policy: None,
            auth: Some(StreamAuthConfig {
                required: true,
                read_roles: Some(HashMap::from([(
                    "testrealm".to_string(),
                    vec!["reader".to_string()],
                )])),
                write_roles: None,
                plugins: None,
            }),
        };

        let api_schema = ApiEventSchema::from(&schema);
        let serialized =
            serde_json::to_value(&api_schema).expect("should serialize api event schema");

        assert!(
            serialized.get("auth").is_none(),
            "api schema must not expose internal auth configuration"
        );
    }

    #[test]
    fn settings_deserialize_with_default_auth_settings_when_missing() {
        let settings: super::Settings = serde_json::from_str(
            r#"{
                "application": {
                    "host": "127.0.0.1",
                    "port": 8080,
                    "base_url": "http://localhost",
                    "static_files_path": "/tmp"
                },
                "notification_backend": {
                    "kind": "in_memory"
                }
            }"#,
        )
        .expect("should deserialize settings");

        assert!(!settings.auth.enabled);
        assert_eq!(settings.auth.timeout_ms, 5_000);
        assert!(settings.auth.admin_roles.is_empty());
    }

    #[test]
    fn settings_deserialize_with_auth_settings_override() {
        let settings: super::Settings = serde_json::from_str(
            r#"{
                "application": {
                    "host": "127.0.0.1",
                    "port": 8080,
                    "base_url": "http://localhost",
                    "static_files_path": "/tmp"
                },
                "notification_backend": {
                    "kind": "in_memory"
                },
                "auth": {
                    "enabled": true,
                    "auth_o_tron_url": "http://auth-o-tron:8080",
                    "jwt_secret": "secret",
                    "admin_roles": {"testrealm": ["admin"]},
                    "timeout_ms": 1000
                }
            }"#,
        )
        .expect("should deserialize settings with auth");

        assert!(settings.auth.enabled);
        assert_eq!(
            settings.auth.auth_o_tron_url,
            "http://auth-o-tron:8080".to_string()
        );
        assert_eq!(settings.auth.jwt_secret, "secret".to_string());
        assert_eq!(
            settings
                .auth
                .admin_roles
                .get("testrealm")
                .map(Vec::as_slice),
            Some(["admin".to_string()].as_slice())
        );
        assert_eq!(settings.auth.timeout_ms, 1000);
    }

    #[test]
    fn test_stream_auth_config_plugins_field() {
        let config: StreamAuthConfig =
            serde_json::from_str(r#"{"required": true, "plugins": ["ecpds"]}"#)
                .expect("should deserialize StreamAuthConfig with plugins");

        assert!(config.required);
        assert_eq!(config.plugins, Some(vec!["ecpds".to_string()]));
    }

    #[test]
    fn test_stream_auth_config_no_plugins() {
        let config: StreamAuthConfig = serde_json::from_str(r#"{"required": true}"#)
            .expect("should deserialize StreamAuthConfig without plugins");

        assert!(config.required);
        assert_eq!(config.plugins, None);
    }
}
