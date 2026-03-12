use super::{
    AuthMode, AuthSettings, EventStoragePolicy, Settings, parse_retention_time_spec,
    parse_size_spec,
};
use crate::notification_backend::{BackendCapabilities, capabilities_for_backend_kind};
use anyhow::{Result, bail};
use std::collections::HashMap;

/// Validates auth configuration
///
/// Enforces mode-specific required fields when auth is enabled.
pub fn validate_auth_settings(auth: &AuthSettings) -> Result<()> {
    if !auth.enabled {
        return Ok(());
    }

    let auth_o_tron_url = auth.auth_o_tron_url.trim();
    let jwt_secret = auth.jwt_secret.trim();

    if auth.timeout_ms == 0 {
        bail!("auth.timeout_ms must be greater than zero");
    }

    if auth.admin_roles.is_empty() {
        bail!("auth.enabled=true requires auth.admin_roles to contain at least one role");
    }
    if auth.admin_roles.iter().any(|role| role.trim().is_empty()) {
        bail!("auth.admin_roles must not contain empty or whitespace-only entries");
    }

    match auth.mode {
        AuthMode::Direct => {
            let has_auth_o_tron = !auth_o_tron_url.is_empty();
            let has_jwt_secret = !jwt_secret.is_empty();

            if !has_auth_o_tron || !has_jwt_secret {
                bail!(
                    "auth.mode=direct requires both auth.auth_o_tron_url and auth.jwt_secret to be configured"
                );
            }
        }
        AuthMode::TrustedProxy => {
            if jwt_secret.is_empty() {
                bail!("auth.mode=trusted_proxy requires auth.jwt_secret to be configured");
            }
        }
    }

    Ok(())
}

/// Validates stream-level auth blocks against global auth mode.
///
/// Stream auth rules are enforceable only when `auth.enabled=true`.
pub fn validate_stream_auth_settings(settings: &Settings) -> Result<()> {
    let Some(schema_map) = settings.notification_schema.as_ref() else {
        return Ok(());
    };

    for (event_type, schema) in schema_map {
        let Some(stream_auth) = schema.auth.as_ref() else {
            continue;
        };

        if stream_auth
            .allowed_roles
            .as_ref()
            .is_some_and(|roles| roles.iter().any(|role| role.trim().is_empty()))
        {
            bail!(
                "Schema '{event_type}' auth.allowed_roles must not contain empty or whitespace-only entries"
            );
        }

        if settings.auth.enabled {
            if !stream_auth.required
                && stream_auth
                    .allowed_roles
                    .as_ref()
                    .is_some_and(|roles| !roles.is_empty())
            {
                bail!(
                    "Schema '{event_type}' sets auth.allowed_roles while auth.required=false; \
                     set auth.required=true or remove auth.allowed_roles"
                );
            }
            continue;
        }

        if stream_auth.required {
            bail!(
                "Schema '{event_type}' sets auth.required=true but auth is globally disabled. Enable auth.enabled=true or remove schema auth config."
            );
        }

        if stream_auth
            .allowed_roles
            .as_ref()
            .is_some_and(|roles| !roles.is_empty())
        {
            bail!(
                "Schema '{event_type}' sets auth.allowed_roles but auth is globally disabled. Enable auth.enabled=true or remove schema auth config."
            );
        }
    }

    Ok(())
}

pub fn validate_schema_storage_policy_support(settings: &Settings) -> Result<()> {
    let kind = settings.notification_backend.kind.as_str();
    let capabilities = capabilities_for_backend_kind(kind)
        .ok_or_else(|| anyhow::anyhow!("Unknown notification_backend kind: {kind}"))?;

    let Some(schema_map) = settings.notification_schema.as_ref() else {
        return Ok(());
    };

    let mut topic_owner_by_base: HashMap<String, String> = HashMap::new();
    for (event_type, schema) in schema_map {
        let Some(topic) = schema.topic.as_ref() else {
            continue;
        };
        let base_key = topic.base.to_ascii_lowercase();
        if let Some(previous_owner) = topic_owner_by_base.get(&base_key) {
            bail!(
                "Schemas '{previous_owner}' and '{event_type}' both define topic base '{}'",
                topic.base
            );
        }
        topic_owner_by_base.insert(base_key, event_type.clone());
    }

    let mut policy_owner_by_base: HashMap<String, String> = HashMap::new();

    for (event_type, schema) in schema_map {
        let Some(policy) = schema.storage_policy.as_ref() else {
            continue;
        };
        let topic = schema.topic.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "Schema '{event_type}' defines storage_policy but has no topic configuration"
            )
        })?;
        let base_key = topic.base.to_ascii_lowercase();
        if let Some(previous_owner) = policy_owner_by_base.get(&base_key) {
            bail!(
                "Schemas '{previous_owner}' and '{event_type}' both define storage_policy for topic base '{}'",
                topic.base
            );
        }
        policy_owner_by_base.insert(base_key, event_type.clone());
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
        let retention = parse_retention_time_spec(retention_time).map_err(|e| {
            anyhow::anyhow!("Schema '{event_type}' storage_policy.retention_time is invalid: {e}")
        })?;
        if retention.is_zero() {
            bail!("Schema '{event_type}' storage_policy.retention_time must be greater than zero");
        }
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
    use super::{
        validate_auth_settings, validate_schema_storage_policy_support,
        validate_stream_auth_settings,
    };
    use crate::configuration::{
        ApplicationSettings, AuthMode, AuthSettings, EventSchema, EventStoragePolicy,
        NotificationBackendSettings, Settings, TopicConfig, WatchEndpointSettings,
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
                topic: Some(TopicConfig {
                    base: event_type.to_ascii_lowercase(),
                    key_order: vec![],
                }),
                endpoint: None,
                identifier: HashMap::new(),
                storage_policy: Some(policy),
                auth: None,
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
            auth: AuthSettings::default(),
        }
    }

    fn basic_settings_with_schema(schema: HashMap<String, EventSchema>) -> Settings {
        Settings {
            application: ApplicationSettings {
                host: "127.0.0.1".to_string(),
                port: 8000,
                base_url: "http://localhost".to_string(),
                static_files_path: "/tmp".to_string(),
            },
            notification_backend: NotificationBackendSettings {
                kind: "in_memory".to_string(),
                in_memory: None,
                jetstream: None,
            },
            logging: None,
            notification_schema: Some(schema),
            watch_endpoint: WatchEndpointSettings::default(),
            auth: AuthSettings::default(),
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
    fn rejects_max_messages_for_in_memory_backend() {
        let settings = settings_with_policy(
            "in_memory",
            EventStoragePolicy {
                max_messages: Some(10),
                ..EventStoragePolicy::default()
            },
            "mars",
        );
        let err = validate_schema_storage_policy_support(&settings)
            .expect_err("in_memory should reject max_messages");
        assert!(err.to_string().contains(
            "Schema 'mars' storage_policy.max_messages is not supported by backend 'in_memory'"
        ));
    }

    #[test]
    fn rejects_retention_time_for_in_memory_backend() {
        let settings = settings_with_policy(
            "in_memory",
            EventStoragePolicy {
                retention_time: Some("1h".to_string()),
                ..EventStoragePolicy::default()
            },
            "mars",
        );
        let err = validate_schema_storage_policy_support(&settings)
            .expect_err("in_memory should reject retention_time");
        assert!(err.to_string().contains(
            "Schema 'mars' storage_policy.retention_time is not supported by backend 'in_memory'"
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
    fn rejects_storage_policy_for_unknown_backend_kind() {
        let settings = settings_with_policy(
            "unknown_backend",
            EventStoragePolicy {
                retention_time: Some("1d".to_string()),
                ..EventStoragePolicy::default()
            },
            "mars",
        );
        let err = validate_schema_storage_policy_support(&settings)
            .expect_err("unknown backend kind must fail");
        assert!(
            err.to_string()
                .contains("Unknown notification_backend kind: unknown_backend")
        );
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
    fn rejects_non_positive_retention_time() {
        let settings = settings_with_policy(
            "jetstream",
            EventStoragePolicy {
                retention_time: Some("0s".to_string()),
                ..EventStoragePolicy::default()
            },
            "mars",
        );
        let err = validate_schema_storage_policy_support(&settings)
            .expect_err("zero retention_time must fail");
        assert!(
            err.to_string()
                .contains("Schema 'mars' storage_policy.retention_time must be greater than zero")
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

    #[test]
    fn rejects_storage_policy_without_topic_configuration() {
        let mut settings = settings_with_policy(
            "jetstream",
            EventStoragePolicy {
                max_messages: Some(10),
                ..EventStoragePolicy::default()
            },
            "mars",
        );
        if let Some(schema_map) = settings.notification_schema.as_mut()
            && let Some(schema) = schema_map.get_mut("mars")
        {
            schema.topic = None;
        }

        let err = validate_schema_storage_policy_support(&settings)
            .expect_err("storage policy without topic must fail");
        assert!(
            err.to_string()
                .contains("Schema 'mars' defines storage_policy but has no topic configuration")
        );
    }

    #[test]
    fn rejects_duplicate_topic_base_across_schemas() {
        let mut schema_map = HashMap::new();
        schema_map.insert(
            "dissemination".to_string(),
            EventSchema {
                payload: None,
                topic: Some(TopicConfig {
                    base: "diss".to_string(),
                    key_order: vec![],
                }),
                endpoint: None,
                identifier: HashMap::new(),
                storage_policy: Some(EventStoragePolicy {
                    max_messages: Some(10),
                    ..EventStoragePolicy::default()
                }),
                auth: None,
            },
        );
        schema_map.insert(
            "diss_alias".to_string(),
            EventSchema {
                payload: None,
                topic: Some(TopicConfig {
                    base: "diss".to_string(),
                    key_order: vec![],
                }),
                endpoint: None,
                identifier: HashMap::new(),
                storage_policy: Some(EventStoragePolicy {
                    max_messages: Some(20),
                    ..EventStoragePolicy::default()
                }),
                auth: None,
            },
        );

        let settings = Settings {
            application: ApplicationSettings {
                host: "127.0.0.1".to_string(),
                port: 8000,
                base_url: "http://localhost".to_string(),
                static_files_path: "/tmp".to_string(),
            },
            notification_backend: NotificationBackendSettings {
                kind: "jetstream".to_string(),
                in_memory: None,
                jetstream: None,
            },
            logging: None,
            notification_schema: Some(schema_map),
            watch_endpoint: WatchEndpointSettings::default(),
            auth: AuthSettings::default(),
        };

        let err = validate_schema_storage_policy_support(&settings)
            .expect_err("duplicate base must fail");
        let message = err.to_string();
        assert!(message.contains("both define topic base 'diss'"));
        assert!(message.contains("'dissemination'"));
        assert!(message.contains("'diss_alias'"));
    }

    #[test]
    fn rejects_duplicate_topic_base_across_schemas_case_insensitive() {
        let mut schema_map = HashMap::new();
        schema_map.insert(
            "schema_a".to_string(),
            EventSchema {
                payload: None,
                topic: Some(TopicConfig {
                    base: "DISS".to_string(),
                    key_order: vec![],
                }),
                endpoint: None,
                identifier: HashMap::new(),
                storage_policy: Some(EventStoragePolicy {
                    max_messages: Some(10),
                    ..EventStoragePolicy::default()
                }),
                auth: None,
            },
        );
        schema_map.insert(
            "schema_b".to_string(),
            EventSchema {
                payload: None,
                topic: Some(TopicConfig {
                    base: "diss".to_string(),
                    key_order: vec![],
                }),
                endpoint: None,
                identifier: HashMap::new(),
                storage_policy: Some(EventStoragePolicy {
                    max_messages: Some(20),
                    ..EventStoragePolicy::default()
                }),
                auth: None,
            },
        );

        let settings = Settings {
            application: ApplicationSettings {
                host: "127.0.0.1".to_string(),
                port: 8000,
                base_url: "http://localhost".to_string(),
                static_files_path: "/tmp".to_string(),
            },
            notification_backend: NotificationBackendSettings {
                kind: "jetstream".to_string(),
                in_memory: None,
                jetstream: None,
            },
            logging: None,
            notification_schema: Some(schema_map),
            watch_endpoint: WatchEndpointSettings::default(),
            auth: AuthSettings::default(),
        };

        let err = validate_schema_storage_policy_support(&settings)
            .expect_err("duplicate base must fail");
        let message = err.to_string();
        assert!(message.contains("both define topic base"));
        assert!(message.contains("'schema_a'"));
        assert!(message.contains("'schema_b'"));
    }

    #[test]
    fn rejects_duplicate_topic_base_even_without_storage_policy() {
        let mut schema_map = HashMap::new();
        schema_map.insert(
            "schema_a".to_string(),
            EventSchema {
                payload: None,
                topic: Some(TopicConfig {
                    base: "shared".to_string(),
                    key_order: vec![],
                }),
                endpoint: None,
                identifier: HashMap::new(),
                storage_policy: None,
                auth: None,
            },
        );
        schema_map.insert(
            "schema_b".to_string(),
            EventSchema {
                payload: None,
                topic: Some(TopicConfig {
                    base: "shared".to_string(),
                    key_order: vec![],
                }),
                endpoint: None,
                identifier: HashMap::new(),
                storage_policy: None,
                auth: None,
            },
        );

        let settings = Settings {
            application: ApplicationSettings {
                host: "127.0.0.1".to_string(),
                port: 8000,
                base_url: "http://localhost".to_string(),
                static_files_path: "/tmp".to_string(),
            },
            notification_backend: NotificationBackendSettings {
                kind: "jetstream".to_string(),
                in_memory: None,
                jetstream: None,
            },
            logging: None,
            notification_schema: Some(schema_map),
            watch_endpoint: WatchEndpointSettings::default(),
            auth: AuthSettings::default(),
        };

        let err = validate_schema_storage_policy_support(&settings)
            .expect_err("duplicate base must fail");
        let message = err.to_string();
        assert!(message.contains("both define topic base 'shared'"));
        assert!(message.contains("'schema_a'"));
        assert!(message.contains("'schema_b'"));
    }

    #[test]
    fn accepts_disabled_auth_without_credentials() {
        let auth = AuthSettings::default();
        assert!(validate_auth_settings(&auth).is_ok());
    }

    #[test]
    fn accepts_enabled_auth_with_auth_o_tron_url_and_jwt_secret() {
        let auth = AuthSettings {
            enabled: true,
            mode: AuthMode::Direct,
            auth_o_tron_url: "http://auth-o-tron:8080".to_string(),
            jwt_secret: "secret".to_string(),
            admin_roles: vec!["admin".to_string()],
            ..AuthSettings::default()
        };
        assert!(validate_auth_settings(&auth).is_ok());
    }

    #[test]
    fn rejects_enabled_auth_with_only_jwt_secret() {
        let auth = AuthSettings {
            enabled: true,
            mode: AuthMode::Direct,
            jwt_secret: "secret".to_string(),
            admin_roles: vec!["admin".to_string()],
            ..AuthSettings::default()
        };
        let err = validate_auth_settings(&auth).expect_err("should fail");
        assert!(err.to_string().contains("auth.mode=direct requires both"));
    }

    #[test]
    fn rejects_enabled_auth_with_only_auth_o_tron_url() {
        let auth = AuthSettings {
            enabled: true,
            mode: AuthMode::Direct,
            auth_o_tron_url: "http://auth-o-tron:8080".to_string(),
            admin_roles: vec!["admin".to_string()],
            ..AuthSettings::default()
        };
        let err = validate_auth_settings(&auth).expect_err("should fail");
        assert!(err.to_string().contains("auth.mode=direct requires both"));
    }

    #[test]
    fn rejects_enabled_auth_without_credentials() {
        let auth = AuthSettings {
            enabled: true,
            mode: AuthMode::Direct,
            admin_roles: vec!["admin".to_string()],
            ..AuthSettings::default()
        };
        let err = validate_auth_settings(&auth).expect_err("should fail");
        assert!(err.to_string().contains("auth.mode=direct requires both"));
    }

    #[test]
    fn rejects_enabled_auth_with_empty_admin_roles() {
        let auth = AuthSettings {
            enabled: true,
            mode: AuthMode::Direct,
            auth_o_tron_url: "http://auth-o-tron:8080".to_string(),
            jwt_secret: "secret".to_string(),
            ..AuthSettings::default()
        };
        let err = validate_auth_settings(&auth).expect_err("should fail");
        assert!(
            err.to_string()
                .contains("auth.enabled=true requires auth.admin_roles")
        );
    }

    #[test]
    fn rejects_enabled_auth_with_whitespace_admin_role() {
        let auth = AuthSettings {
            enabled: true,
            mode: AuthMode::Direct,
            auth_o_tron_url: "http://auth-o-tron:8080".to_string(),
            jwt_secret: "secret".to_string(),
            admin_roles: vec!["admin".to_string(), "   ".to_string()],
            ..AuthSettings::default()
        };
        let err = validate_auth_settings(&auth).expect_err("should fail");
        assert!(
            err.to_string()
                .contains("auth.admin_roles must not contain empty or whitespace-only entries")
        );
    }

    #[test]
    fn rejects_enabled_auth_with_zero_timeout() {
        let auth = AuthSettings {
            enabled: true,
            mode: AuthMode::Direct,
            auth_o_tron_url: "http://auth-o-tron:8080".to_string(),
            jwt_secret: "secret".to_string(),
            admin_roles: vec!["admin".to_string()],
            timeout_ms: 0,
        };
        let err = validate_auth_settings(&auth).expect_err("should fail");
        assert!(
            err.to_string()
                .contains("auth.timeout_ms must be greater than zero")
        );
    }

    #[test]
    fn rejects_enabled_auth_with_whitespace_auth_o_tron_url() {
        let auth = AuthSettings {
            enabled: true,
            mode: AuthMode::Direct,
            auth_o_tron_url: "   ".to_string(),
            jwt_secret: "secret".to_string(),
            admin_roles: vec!["admin".to_string()],
            ..AuthSettings::default()
        };
        let err = validate_auth_settings(&auth).expect_err("should fail");
        assert!(err.to_string().contains("auth.mode=direct requires both"));
    }

    #[test]
    fn rejects_enabled_auth_with_whitespace_jwt_secret() {
        let auth = AuthSettings {
            enabled: true,
            mode: AuthMode::Direct,
            auth_o_tron_url: "http://auth-o-tron:8080".to_string(),
            jwt_secret: "   ".to_string(),
            admin_roles: vec!["admin".to_string()],
            ..AuthSettings::default()
        };
        let err = validate_auth_settings(&auth).expect_err("should fail");
        assert!(err.to_string().contains("auth.mode=direct requires both"));
    }

    #[test]
    fn rejects_stream_auth_required_when_global_auth_is_disabled() {
        let mut schema_map = HashMap::new();
        schema_map.insert(
            "mars".to_string(),
            EventSchema {
                payload: None,
                topic: Some(TopicConfig {
                    base: "mars".to_string(),
                    key_order: vec![],
                }),
                endpoint: None,
                identifier: HashMap::new(),
                storage_policy: None,
                auth: Some(crate::configuration::StreamAuthConfig {
                    required: true,
                    allowed_roles: None,
                }),
            },
        );

        let settings = basic_settings_with_schema(schema_map);
        let err = validate_stream_auth_settings(&settings)
            .expect_err("stream auth.required=true should fail when auth is disabled");
        assert!(
            err.to_string()
                .contains("Schema 'mars' sets auth.required=true but auth is globally disabled")
        );
    }

    #[test]
    fn rejects_stream_auth_allowed_roles_when_global_auth_is_disabled() {
        let mut schema_map = HashMap::new();
        schema_map.insert(
            "diss".to_string(),
            EventSchema {
                payload: None,
                topic: Some(TopicConfig {
                    base: "diss".to_string(),
                    key_order: vec![],
                }),
                endpoint: None,
                identifier: HashMap::new(),
                storage_policy: None,
                auth: Some(crate::configuration::StreamAuthConfig {
                    required: false,
                    allowed_roles: Some(vec!["admin".to_string()]),
                }),
            },
        );

        let settings = basic_settings_with_schema(schema_map);
        let err = validate_stream_auth_settings(&settings)
            .expect_err("stream auth.allowed_roles should fail when auth is disabled");
        assert!(
            err.to_string()
                .contains("Schema 'diss' sets auth.allowed_roles but auth is globally disabled")
        );
    }

    #[test]
    fn accepts_stream_auth_when_global_auth_is_enabled() {
        let mut schema_map = HashMap::new();
        schema_map.insert(
            "events".to_string(),
            EventSchema {
                payload: None,
                topic: Some(TopicConfig {
                    base: "events".to_string(),
                    key_order: vec![],
                }),
                endpoint: None,
                identifier: HashMap::new(),
                storage_policy: None,
                auth: Some(crate::configuration::StreamAuthConfig {
                    required: true,
                    allowed_roles: Some(vec!["reader".to_string()]),
                }),
            },
        );

        let mut settings = basic_settings_with_schema(schema_map);
        settings.auth.enabled = true;
        settings.auth.mode = AuthMode::Direct;
        settings.auth.auth_o_tron_url = "http://auth-o-tron:8080".to_string();
        settings.auth.jwt_secret = "secret".to_string();
        settings.auth.admin_roles = vec!["admin".to_string()];

        validate_stream_auth_settings(&settings)
            .expect("stream auth should be accepted when auth is enabled");
    }

    #[test]
    fn accepts_empty_stream_allowed_roles_when_global_auth_is_enabled() {
        let mut schema_map = HashMap::new();
        schema_map.insert(
            "events".to_string(),
            EventSchema {
                payload: None,
                topic: Some(TopicConfig {
                    base: "events".to_string(),
                    key_order: vec![],
                }),
                endpoint: None,
                identifier: HashMap::new(),
                storage_policy: None,
                auth: Some(crate::configuration::StreamAuthConfig {
                    required: true,
                    allowed_roles: Some(vec![]),
                }),
            },
        );

        let mut settings = basic_settings_with_schema(schema_map);
        settings.auth.enabled = true;
        settings.auth.mode = AuthMode::Direct;
        settings.auth.auth_o_tron_url = "http://auth-o-tron:8080".to_string();
        settings.auth.jwt_secret = "secret".to_string();
        settings.auth.admin_roles = vec!["admin".to_string()];

        validate_stream_auth_settings(&settings)
            .expect("empty stream allowed_roles should be accepted");
    }

    #[test]
    fn rejects_stream_allowed_roles_with_whitespace_entry_when_auth_enabled() {
        let mut schema_map = HashMap::new();
        schema_map.insert(
            "events".to_string(),
            EventSchema {
                payload: None,
                topic: Some(TopicConfig {
                    base: "events".to_string(),
                    key_order: vec![],
                }),
                endpoint: None,
                identifier: HashMap::new(),
                storage_policy: None,
                auth: Some(crate::configuration::StreamAuthConfig {
                    required: true,
                    allowed_roles: Some(vec!["admin".to_string(), " ".to_string()]),
                }),
            },
        );

        let mut settings = basic_settings_with_schema(schema_map);
        settings.auth.enabled = true;
        settings.auth.mode = AuthMode::Direct;
        settings.auth.auth_o_tron_url = "http://auth-o-tron:8080".to_string();
        settings.auth.jwt_secret = "secret".to_string();
        settings.auth.admin_roles = vec!["admin".to_string()];

        let err = validate_stream_auth_settings(&settings).expect_err("should fail");
        assert!(
            err.to_string()
                .contains("auth.allowed_roles must not contain empty or whitespace-only entries")
        );
    }

    #[test]
    fn rejects_allowed_roles_when_required_is_false_and_auth_enabled() {
        let mut schema_map = HashMap::new();
        schema_map.insert(
            "events".to_string(),
            EventSchema {
                payload: None,
                topic: Some(TopicConfig {
                    base: "events".to_string(),
                    key_order: vec![],
                }),
                endpoint: None,
                identifier: HashMap::new(),
                storage_policy: None,
                auth: Some(crate::configuration::StreamAuthConfig {
                    required: false,
                    allowed_roles: Some(vec!["reader".to_string()]),
                }),
            },
        );

        let mut settings = basic_settings_with_schema(schema_map);
        settings.auth.enabled = true;
        settings.auth.mode = AuthMode::Direct;
        settings.auth.auth_o_tron_url = "http://auth-o-tron:8080".to_string();
        settings.auth.jwt_secret = "secret".to_string();
        settings.auth.admin_roles = vec!["admin".to_string()];

        let err = validate_stream_auth_settings(&settings).expect_err("should fail");
        assert!(
            err.to_string()
                .contains("auth.allowed_roles while auth.required=false")
        );
    }

    #[test]
    fn accepts_enabled_trusted_proxy_auth_with_required_fields() {
        let auth = AuthSettings {
            enabled: true,
            mode: AuthMode::TrustedProxy,
            jwt_secret: "secret".to_string(),
            admin_roles: vec!["admin".to_string()],
            ..AuthSettings::default()
        };
        assert!(validate_auth_settings(&auth).is_ok());
    }

    #[test]
    fn rejects_enabled_trusted_proxy_auth_without_jwt_secret() {
        let auth = AuthSettings {
            enabled: true,
            mode: AuthMode::TrustedProxy,
            admin_roles: vec!["admin".to_string()],
            ..AuthSettings::default()
        };
        let err = validate_auth_settings(&auth).expect_err("should fail");
        assert!(
            err.to_string()
                .contains("auth.mode=trusted_proxy requires auth.jwt_secret")
        );
    }
}
