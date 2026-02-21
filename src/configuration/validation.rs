use super::{EventStoragePolicy, Settings, parse_duration_spec, parse_size_spec};
use crate::notification_backend::{BackendCapabilities, capabilities_for_backend_kind};
use anyhow::{Result, bail};

pub fn validate_schema_storage_policy_support(settings: &Settings) -> Result<()> {
    let kind = settings.notification_backend.kind.as_str();
    let capabilities = capabilities_for_backend_kind(kind)
        .ok_or_else(|| anyhow::anyhow!("Unknown notification_backend kind: {kind}"))?;

    let Some(schema_map) = settings.notification_schema.as_ref() else {
        return Ok(());
    };

    for (event_type, schema) in schema_map {
        let Some(policy) = schema.storage_policy.as_ref() else {
            continue;
        };
        validate_policy_fields(kind, event_type, policy, capabilities)?;
    }

    Ok(())
}

fn validate_policy_fields(
    backend_kind: &str,
    event_type: &str,
    policy: &EventStoragePolicy,
    capabilities: BackendCapabilities,
) -> Result<()> {
    if let Some(retention_time) = policy.retention_time.as_deref() {
        parse_duration_spec(retention_time).map_err(|e| {
            anyhow::anyhow!("Schema '{event_type}' storage_policy.retention_time is invalid: {e}")
        })?;
    }
    if let Some(max_size) = policy.max_size.as_deref() {
        parse_size_spec(max_size).map_err(|e| {
            anyhow::anyhow!("Schema '{event_type}' storage_policy.max_size is invalid: {e}")
        })?;
    }
    if let Some(max_messages) = policy.max_messages
        && max_messages <= 0
    {
        bail!("Schema '{event_type}' storage_policy.max_messages must be greater than zero");
    }

    if policy.retention_time.is_some() && !capabilities.retention_time {
        bail!(
            "Schema '{event_type}' storage_policy.retention_time is not supported by backend '{backend_kind}'"
        );
    }
    if policy.max_messages.is_some() && !capabilities.max_messages {
        bail!(
            "Schema '{event_type}' storage_policy.max_messages is not supported by backend '{backend_kind}'"
        );
    }
    if policy.max_size.is_some() && !capabilities.max_size {
        bail!(
            "Schema '{event_type}' storage_policy.max_size is not supported by backend '{backend_kind}'"
        );
    }
    if policy.allow_duplicates.is_some() && !capabilities.allow_duplicates {
        bail!(
            "Schema '{event_type}' storage_policy.allow_duplicates is not supported by backend '{backend_kind}'"
        );
    }
    if policy.compression.is_some() && !capabilities.compression {
        bail!(
            "Schema '{event_type}' storage_policy.compression is not supported by backend '{backend_kind}'"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_schema_storage_policy_support;
    use crate::configuration::{
        ApplicationSettings, EventSchema, EventStoragePolicy, NotificationBackendSettings,
        Settings, WatchEndpointSettings,
    };
    use std::collections::HashMap;

    fn settings_with_policy(
        backend_kind: &str,
        policy: EventStoragePolicy,
        event_type: &str,
    ) -> Settings {
        let mut schema = HashMap::new();
        schema.insert(
            event_type.to_string(),
            EventSchema {
                payload: None,
                topic: None,
                endpoint: None,
                identifier: HashMap::new(),
                storage_policy: Some(policy),
            },
        );

        Settings {
            application: ApplicationSettings {
                host: "127.0.0.1".to_string(),
                port: 8000,
                base_url: "http://localhost".to_string(),
                static_files_path: "/tmp".to_string(),
            },
            notification_backend: NotificationBackendSettings {
                kind: backend_kind.to_string(),
                in_memory: None,
                jetstream: None,
            },
            logging: None,
            notification_schema: Some(schema),
            watch_endpoint: WatchEndpointSettings::default(),
        }
    }

    #[test]
    fn rejects_unsupported_field_for_in_memory_backend() {
        let settings = settings_with_policy(
            "in_memory",
            EventStoragePolicy {
                compression: Some(true),
                ..EventStoragePolicy::default()
            },
            "mars",
        );
        let err = validate_schema_storage_policy_support(&settings)
            .expect_err("in_memory should reject compression");
        assert!(err.to_string().contains(
            "Schema 'mars' storage_policy.compression is not supported by backend 'in_memory'"
        ));
    }

    #[test]
    fn accepts_supported_fields_for_jetstream_backend() {
        let settings = settings_with_policy(
            "jetstream",
            EventStoragePolicy {
                retention_time: Some("7d".to_string()),
                max_messages: Some(1000),
                max_size: Some("1Gi".to_string()),
                allow_duplicates: Some(true),
                compression: Some(true),
            },
            "dissemination",
        );
        validate_schema_storage_policy_support(&settings)
            .expect("jetstream should accept all storage policy fields");
    }

    #[test]
    fn rejects_invalid_retention_time_format() {
        let settings = settings_with_policy(
            "jetstream",
            EventStoragePolicy {
                retention_time: Some("10x".to_string()),
                ..EventStoragePolicy::default()
            },
            "mars",
        );
        let err = validate_schema_storage_policy_support(&settings)
            .expect_err("invalid retention_time must fail");
        assert!(
            err.to_string()
                .contains("Schema 'mars' storage_policy.retention_time is invalid:")
        );
    }

    #[test]
    fn rejects_invalid_max_size_format() {
        let settings = settings_with_policy(
            "jetstream",
            EventStoragePolicy {
                max_size: Some("10m".to_string()),
                ..EventStoragePolicy::default()
            },
            "mars",
        );
        let err = validate_schema_storage_policy_support(&settings)
            .expect_err("invalid max_size must fail");
        assert!(
            err.to_string()
                .contains("Schema 'mars' storage_policy.max_size is invalid:")
        );
    }

    #[test]
    fn rejects_non_positive_max_messages() {
        let settings = settings_with_policy(
            "jetstream",
            EventStoragePolicy {
                max_messages: Some(0),
                ..EventStoragePolicy::default()
            },
            "mars",
        );
        let err = validate_schema_storage_policy_support(&settings)
            .expect_err("non-positive max_messages must fail");
        assert!(
            err.to_string()
                .contains("Schema 'mars' storage_policy.max_messages must be greater than zero")
        );
    }
}
