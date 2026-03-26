// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use crate::configuration::{
    JetStreamDiscardPolicy, JetStreamRetentionPolicy, JetStreamSettings, JetStreamStorageType,
    NotificationBackendSettings, parse_retention_time_spec,
};
use anyhow::{Result, bail};
use std::time::Duration;

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
    /// Default stream retention window.
    pub retention_time: Option<Duration>,
    /// Storage type for streams
    pub storage_type: JetStreamStorageType,
    /// Number of replicas for high availability
    pub replicas: Option<usize>,
    /// Retention policy
    pub retention_policy: JetStreamRetentionPolicy,
    /// Discard policy when limits are reached
    pub discard_policy: JetStreamDiscardPolicy,
    /// Enable automatic reconnection on failures
    pub enable_auto_reconnect: bool,
    /// Maximum reconnection attempts before giving up temporarily
    pub max_reconnect_attempts: u32,
    /// Base delay between reconnection attempts in milliseconds
    pub reconnect_delay_ms: u64,
    /// Maximum publish attempts when transient channel-closed errors occur.
    pub publish_retry_attempts: u32,
    /// Base backoff for publish retries on channel-closed errors.
    pub publish_retry_base_delay_ms: u64,
}

impl JetStreamConfig {
    /// Create JetStreamConfig from application configuration
    /// Merges configuration file settings
    /// Environment variables take precedence over config file values
    pub fn from_backend_settings(settings: &NotificationBackendSettings) -> Result<Self> {
        let js_settings = settings.jetstream.as_ref();
        let retention_time = js_settings
            .and_then(|js| js.retention_time.as_deref())
            .map(|value| {
                parse_retention_time_spec(value).map_err(|e| {
                    anyhow::anyhow!("notification_backend.jetstream.retention_time is invalid: {e}")
                })
            })
            .transpose()?;

        Ok(Self {
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
            retention_time,
            storage_type: get_storage_type(js_settings),
            replicas: js_settings.and_then(|js| js.replicas),
            retention_policy: get_retention_policy(js_settings),
            discard_policy: get_discard_policy(js_settings),
            enable_auto_reconnect: js_settings
                .and_then(|js| js.enable_auto_reconnect)
                .unwrap_or(true),
            max_reconnect_attempts: js_settings
                .and_then(|js| js.max_reconnect_attempts)
                .unwrap_or(5),
            reconnect_delay_ms: js_settings
                .and_then(|js| js.reconnect_delay_ms)
                .unwrap_or(2000),
            publish_retry_attempts: js_settings
                .and_then(|js| js.publish_retry_attempts)
                .unwrap_or(5),
            publish_retry_base_delay_ms: js_settings
                .and_then(|js| js.publish_retry_base_delay_ms)
                .unwrap_or(150),
        })
    }

    /// Validate JetStream settings that should fail fast at startup.
    pub fn validate(&self) -> Result<()> {
        if self.nats_url.trim().is_empty() {
            bail!("notification_backend.jetstream.nats_url must not be empty");
        }

        if self.timeout_seconds == 0 {
            bail!("notification_backend.jetstream.timeout_seconds must be > 0");
        }

        if self.retry_attempts == 0 {
            // Connection code defensively clamps with `.max(1)`, but config keeps
            // strict semantics so users cannot rely on implicit clamping.
            bail!("notification_backend.jetstream.retry_attempts must be > 0");
        }

        if self.reconnect_delay_ms == 0 {
            bail!("notification_backend.jetstream.reconnect_delay_ms must be > 0");
        }
        if let Some(retention_time) = self.retention_time
            && retention_time.is_zero()
        {
            bail!("notification_backend.jetstream.retention_time must be > 0");
        }
        if self.publish_retry_attempts == 0 {
            bail!("notification_backend.jetstream.publish_retry_attempts must be > 0");
        }
        if self.publish_retry_base_delay_ms == 0 {
            bail!("notification_backend.jetstream.publish_retry_base_delay_ms must be > 0");
        }

        Ok(())
    }
}

fn get_storage_type(settings: Option<&JetStreamSettings>) -> JetStreamStorageType {
    settings
        .and_then(|js| js.storage_type.clone())
        .unwrap_or(JetStreamStorageType::File)
}

fn get_retention_policy(settings: Option<&JetStreamSettings>) -> JetStreamRetentionPolicy {
    settings
        .and_then(|js| js.retention_policy.clone())
        .unwrap_or(JetStreamRetentionPolicy::Limits)
}

fn get_discard_policy(settings: Option<&JetStreamSettings>) -> JetStreamDiscardPolicy {
    settings
        .and_then(|js| js.discard_policy.clone())
        .unwrap_or(JetStreamDiscardPolicy::Old)
}

#[cfg(test)]
mod tests {
    use super::JetStreamConfig;
    use crate::configuration::{
        JetStreamDiscardPolicy, JetStreamRetentionPolicy, JetStreamSettings, JetStreamStorageType,
        NotificationBackendSettings,
    };

    fn base_config() -> JetStreamConfig {
        JetStreamConfig {
            nats_url: "nats://localhost:4222".to_string(),
            timeout_seconds: 30,
            retry_attempts: 3,
            token: None,
            max_messages: None,
            max_bytes: None,
            retention_time: None,
            storage_type: JetStreamStorageType::File,
            replicas: None,
            retention_policy: JetStreamRetentionPolicy::Limits,
            discard_policy: JetStreamDiscardPolicy::Old,
            enable_auto_reconnect: true,
            max_reconnect_attempts: 5,
            reconnect_delay_ms: 2000,
            publish_retry_attempts: 5,
            publish_retry_base_delay_ms: 150,
        }
    }

    #[test]
    fn validate_accepts_valid_configuration() {
        let cfg = base_config();
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn validate_rejects_empty_nats_url() {
        let mut cfg = base_config();
        cfg.nats_url = " ".to_string();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_zero_timeout() {
        let mut cfg = base_config();
        cfg.timeout_seconds = 0;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_zero_retry_attempts() {
        let mut cfg = base_config();
        cfg.retry_attempts = 0;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_zero_reconnect_delay() {
        let mut cfg = base_config();
        cfg.reconnect_delay_ms = 0;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_zero_publish_retry_attempts() {
        let mut cfg = base_config();
        cfg.publish_retry_attempts = 0;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_zero_publish_retry_base_delay() {
        let mut cfg = base_config();
        cfg.publish_retry_base_delay_ms = 0;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn from_backend_settings_uses_typed_defaults_for_policy_fields() {
        let backend_settings = NotificationBackendSettings {
            kind: "jetstream".to_string(),
            in_memory: None,
            jetstream: Some(JetStreamSettings {
                nats_url: None,
                token: None,
                timeout_seconds: None,
                retry_attempts: None,
                max_messages: None,
                max_bytes: None,
                retention_time: None,
                storage_type: None,
                replicas: None,
                retention_policy: None,
                discard_policy: None,
                enable_auto_reconnect: None,
                max_reconnect_attempts: None,
                reconnect_delay_ms: None,
                publish_retry_attempts: None,
                publish_retry_base_delay_ms: None,
            }),
        };

        let cfg = JetStreamConfig::from_backend_settings(&backend_settings)
            .expect("defaults should build config");
        assert!(matches!(cfg.storage_type, JetStreamStorageType::File));
        assert!(matches!(
            cfg.retention_policy,
            JetStreamRetentionPolicy::Limits
        ));
        assert!(matches!(cfg.discard_policy, JetStreamDiscardPolicy::Old));
    }

    #[test]
    fn from_backend_settings_parses_retention_time_duration() {
        let backend_settings = NotificationBackendSettings {
            kind: "jetstream".to_string(),
            in_memory: None,
            jetstream: Some(JetStreamSettings {
                nats_url: None,
                token: None,
                timeout_seconds: None,
                retry_attempts: None,
                max_messages: None,
                max_bytes: None,
                retention_time: Some("2h".to_string()),
                storage_type: None,
                replicas: None,
                retention_policy: None,
                discard_policy: None,
                enable_auto_reconnect: None,
                max_reconnect_attempts: None,
                reconnect_delay_ms: None,
                publish_retry_attempts: None,
                publish_retry_base_delay_ms: None,
            }),
        };

        let cfg = JetStreamConfig::from_backend_settings(&backend_settings)
            .expect("valid duration should parse");
        assert_eq!(
            cfg.retention_time
                .expect("retention_time should be present")
                .as_secs(),
            7200
        );
    }

    #[test]
    fn from_backend_settings_rejects_invalid_retention_time() {
        let backend_settings = NotificationBackendSettings {
            kind: "jetstream".to_string(),
            in_memory: None,
            jetstream: Some(JetStreamSettings {
                nats_url: None,
                token: None,
                timeout_seconds: None,
                retry_attempts: None,
                max_messages: None,
                max_bytes: None,
                retention_time: Some("10x".to_string()),
                storage_type: None,
                replicas: None,
                retention_policy: None,
                discard_policy: None,
                enable_auto_reconnect: None,
                max_reconnect_attempts: None,
                reconnect_delay_ms: None,
                publish_retry_attempts: None,
                publish_retry_base_delay_ms: None,
            }),
        };

        let err = JetStreamConfig::from_backend_settings(&backend_settings)
            .expect_err("invalid retention time must fail");
        assert!(
            err.to_string()
                .contains("notification_backend.jetstream.retention_time is invalid:")
        );
    }
}
