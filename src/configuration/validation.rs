// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use super::{
    AuthMode, AuthSettings, EventStoragePolicy, Settings, parse_retention_time_spec,
    parse_size_spec,
};
use crate::notification_backend::{BackendCapabilities, capabilities_for_backend_kind};
use anyhow::{Result, bail};
use std::collections::HashMap;

/// Validates a realm → roles map: rejects empty, whitespace-only, and whitespace-padded entries.
fn validate_realm_roles(
    field_name: &str,
    realm_roles: &HashMap<String, Vec<String>>,
) -> Result<()> {
    for (realm, roles) in realm_roles {
        if realm.trim().is_empty() {
            bail!("{field_name} must not contain empty or whitespace-only realm keys");
        }
        if realm != realm.trim() {
            bail!("{field_name} realm '{realm}' must not have leading or trailing whitespace");
        }
        if roles.is_empty() {
            bail!("{field_name} realm '{realm}' must have at least one role");
        }
        for role in roles {
            if role.trim().is_empty() {
                bail!(
                    "{field_name} realm '{realm}' must not contain empty or whitespace-only role entries"
                );
            }
            if role != role.trim() {
                bail!(
                    "{field_name} realm '{realm}' role '{role}' must not have leading or trailing whitespace"
                );
            }
        }
    }
    Ok(())
}

/// Validates auth configuration
///
/// Enforces mode-specific required fields when auth is enabled.
pub fn validate_auth_settings(auth: &AuthSettings) -> Result<()> {
    if !auth.enabled {
        return Ok(());
    }

    if auth.auth_o_tron_url != auth.auth_o_tron_url.trim() {
        bail!("auth.auth_o_tron_url must not have leading or trailing whitespace");
    }
    if auth.jwt_secret != auth.jwt_secret.trim() {
        bail!("auth.jwt_secret must not have leading or trailing whitespace");
    }

    let auth_o_tron_url = auth.auth_o_tron_url.trim();
    let jwt_secret = auth.jwt_secret.trim();

    if auth.timeout_ms == 0 {
        bail!("auth.timeout_ms must be greater than zero");
    }

    if auth.admin_roles.is_empty() {
        bail!("auth.enabled=true requires auth.admin_roles to contain at least one realm");
    }
    for (realm, roles) in &auth.admin_roles {
        if roles.is_empty() {
            bail!("auth.admin_roles realm '{realm}' must have at least one role");
        }
    }
    validate_realm_roles("auth.admin_roles", &auth.admin_roles)?;

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

/// Plugin names this codebase recognizes for the per-stream
/// `auth.plugins` list. Add new plugins here AND to
/// [`plugin_required_feature`] if they are feature-gated.
const KNOWN_PLUGINS: &[&str] = &["ecpds"];

/// Returns the Cargo feature flag required by a known plugin, or `None`
/// if the plugin is always available regardless of feature configuration.
fn plugin_required_feature(name: &str) -> Option<&'static str> {
    match name {
        "ecpds" => Some("ecpds"),
        _ => None,
    }
}

/// Compile-time table of Cargo features this binary knows about, paired
/// with whether each is currently enabled. Add a new row when a new
/// feature-gated plugin is introduced.
const COMPILED_FEATURES: &[(&str, bool)] = &[("ecpds", cfg!(feature = "ecpds"))];

/// Returns whether a given Cargo feature flag is currently compiled into
/// this binary. Used by [`validate_stream_plugin_settings`] to fail-close
/// when a stream references a plugin whose feature is off.
fn feature_enabled(name: &str) -> bool {
    COMPILED_FEATURES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, on)| *on)
        .unwrap_or(false)
}

/// Validates per-stream `auth.plugins` lists fail-closed.
///
/// Rejects:
/// - empty plugin lists (operators should omit the field instead),
/// - unknown plugin names (typos),
/// - plugins whose required Cargo feature is not compiled in
///   (otherwise the plugin's check would silently not run),
/// - plugins on a stream where `auth.required` is `false`
///   (plugins only run after stream-level auth passes, so this would
///   never execute the plugin).
///
/// This function is **always compiled** (regardless of feature flags)
/// so that misconfigured deployments fail at startup instead of silently
/// running without the expected authorization plugin.
pub fn validate_stream_plugin_settings(settings: &Settings) -> Result<()> {
    let Some(schema_map) = settings.notification_schema.as_ref() else {
        return Ok(());
    };

    for (event_type, schema) in schema_map {
        let Some(stream_auth) = schema.auth.as_ref() else {
            continue;
        };
        let Some(plugins) = stream_auth.plugins.as_ref() else {
            continue;
        };

        if plugins.is_empty() {
            bail!(
                "Schema '{event_type}' auth.plugins must not be empty; \
                 omit the field instead of setting plugins: []"
            );
        }

        for plugin in plugins {
            if !KNOWN_PLUGINS.contains(&plugin.as_str()) {
                bail!(
                    "Schema '{event_type}' auth.plugins references unknown plugin '{plugin}'. \
                     Known plugins: {KNOWN_PLUGINS:?}"
                );
            }
            if let Some(required_feature) = plugin_required_feature(plugin)
                && !feature_enabled(required_feature)
            {
                bail!(
                    "Schema '{event_type}' auth.plugins references plugin '{plugin}', \
                     which requires the '{required_feature}' Cargo feature. \
                     Rebuild aviso-server with `--features {required_feature}` or \
                     remove the plugin from this stream's auth config."
                );
            }
        }

        if !stream_auth.required {
            bail!(
                "Schema '{event_type}' auth.plugins is set but auth.required is false. \
                 Plugins only run after stream-level auth passes; \
                 set auth.required=true or remove the plugins list."
            );
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

        if let Some(read_roles) = &stream_auth.read_roles {
            if read_roles.is_empty() {
                bail!(
                    "Schema '{event_type}' auth.read_roles must not be an empty map; \
                     remove the field to allow any authenticated user to read"
                );
            }
            validate_realm_roles(
                &format!("Schema '{event_type}' auth.read_roles"),
                read_roles,
            )?;
        }
        if let Some(write_roles) = &stream_auth.write_roles {
            if write_roles.is_empty() {
                bail!(
                    "Schema '{event_type}' auth.write_roles must not be an empty map; \
                     remove the field to restrict writes to admins only"
                );
            }
            validate_realm_roles(
                &format!("Schema '{event_type}' auth.write_roles"),
                write_roles,
            )?;
        }

        let has_roles = stream_auth
            .read_roles
            .as_ref()
            .is_some_and(|r| !r.is_empty())
            || stream_auth
                .write_roles
                .as_ref()
                .is_some_and(|r| !r.is_empty());

        if settings.auth.enabled {
            if !stream_auth.required && has_roles {
                bail!(
                    "Schema '{event_type}' sets auth roles while auth.required=false; \
                     set auth.required=true or remove the role lists"
                );
            }
            continue;
        }

        if stream_auth.required {
            bail!(
                "Schema '{event_type}' sets auth.required=true but auth is globally disabled. Enable auth.enabled=true or remove schema auth config."
            );
        }

        if has_roles {
            bail!(
                "Schema '{event_type}' sets auth roles but auth is globally disabled. Enable auth.enabled=true or remove schema auth config."
            );
        }
    }

    Ok(())
}

pub fn validate_metrics_settings(settings: &Settings) -> Result<()> {
    let metrics = &settings.metrics;
    if !metrics.enabled {
        return Ok(());
    }
    let port = metrics
        .port
        .ok_or_else(|| anyhow::anyhow!("metrics.port is required when metrics.enabled=true"))?;
    if port == 0 {
        bail!("metrics.port must not be 0");
    }
    if port == settings.application.port {
        bail!("metrics.port must differ from application.port");
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

#[cfg(feature = "ecpds")]
pub fn validate_ecpds_settings(settings: &Settings) -> Result<()> {
    let ecpds_streams: Vec<&str> = settings
        .notification_schema
        .as_ref()
        .map(|schema| {
            schema
                .iter()
                .filter(|(_, event_schema)| {
                    event_schema
                        .auth
                        .as_ref()
                        .and_then(|a| a.plugins.as_ref())
                        .map(|plugins| plugins.iter().any(|p| p == "ecpds"))
                        .unwrap_or(false)
                })
                .map(|(name, _)| name.as_str())
                .collect()
        })
        .unwrap_or_default();

    if ecpds_streams.is_empty() {
        return Ok(());
    }

    let ecpds_config = settings.ecpds.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "Streams {:?} reference the 'ecpds' plugin but no 'ecpds' configuration section was found",
            ecpds_streams
        )
    })?;

    if ecpds_config.servers.is_empty() {
        bail!("ecpds.servers must contain at least one server URL");
    }
    for (i, server) in ecpds_config.servers.iter().enumerate() {
        if server.trim().is_empty() {
            bail!("ecpds.servers[{i}] must not be empty or whitespace");
        }
        let parsed = reqwest::Url::parse(server).map_err(|e| {
            anyhow::anyhow!("ecpds.servers[{i}] '{server}' is not a valid URL: {e}")
        })?;
        match parsed.scheme() {
            "http" | "https" => {}
            other => bail!(
                "ecpds.servers[{i}] '{server}' has unsupported scheme '{other}'; \
                 only 'http' and 'https' are accepted"
            ),
        }
        if parsed.query().is_some() {
            bail!(
                "ecpds.servers[{i}] '{server}' must not contain a query string; \
                 the plugin appends '?id=<username>' itself"
            );
        }
        if parsed.fragment().is_some() {
            bail!("ecpds.servers[{i}] '{server}' must not contain a URL fragment");
        }
    }
    if ecpds_config.username.is_empty() {
        bail!("ecpds.username must not be empty");
    }
    if ecpds_config.password.is_empty() {
        bail!("ecpds.password must not be empty");
    }
    if ecpds_config.target_field.is_empty() {
        bail!("ecpds.target_field must not be empty");
    }
    if ecpds_config.match_key.is_empty() {
        bail!("ecpds.match_key must not be empty");
    }
    if ecpds_config
        .match_key
        .chars()
        .any(|c| c.is_whitespace() || c == '/' || c == '\0')
    {
        bail!(
            "ecpds.match_key '{}' must be a single bare identifier name; \
             whitespace, '/' and NUL are not allowed",
            ecpds_config.match_key
        );
    }
    if ecpds_config.cache_ttl_seconds == 0 {
        bail!("ecpds.cache_ttl_seconds must be greater than zero");
    }
    if ecpds_config.max_entries == 0 {
        bail!("ecpds.max_entries must be greater than zero");
    }
    if ecpds_config.request_timeout_seconds == 0 {
        bail!("ecpds.request_timeout_seconds must be greater than zero");
    }
    if ecpds_config.connect_timeout_seconds == 0 {
        bail!("ecpds.connect_timeout_seconds must be greater than zero");
    }

    if let Some(schema) = &settings.notification_schema {
        for stream_name in &ecpds_streams {
            let Some(event_schema) = schema.get(*stream_name) else {
                continue;
            };
            let key_order = event_schema
                .topic
                .as_ref()
                .map(|t| t.key_order.as_slice())
                .unwrap_or_default();
            if !key_order.contains(&ecpds_config.match_key) {
                bail!(
                    "ecpds.match_key '{}' not found in key_order {:?} for stream '{}'",
                    ecpds_config.match_key,
                    key_order,
                    stream_name
                );
            }
            match event_schema.identifier.get(&ecpds_config.match_key) {
                None => bail!(
                    "ecpds.match_key '{}' has no identifier rule defined in schema '{}'",
                    ecpds_config.match_key,
                    stream_name
                ),
                Some(field) if !field.is_required() => bail!(
                    "ecpds.match_key '{}' must be required: true in schema '{}' \
                     so the value is guaranteed to be present before the plugin runs",
                    ecpds_config.match_key,
                    stream_name
                ),
                Some(_) => {}
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        validate_auth_settings, validate_metrics_settings, validate_schema_storage_policy_support,
        validate_stream_auth_settings, validate_stream_plugin_settings,
    };
    use crate::configuration::{
        ApplicationSettings, AuthMode, AuthSettings, EventSchema, EventStoragePolicy,
        MetricsSettings, NotificationBackendSettings, Settings, TopicConfig, WatchEndpointSettings,
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
            metrics: MetricsSettings::default(),
            ecpds: None,
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
            metrics: MetricsSettings::default(),
            ecpds: None,
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
            metrics: MetricsSettings::default(),
            ecpds: None,
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
            metrics: MetricsSettings::default(),
            ecpds: None,
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
            metrics: MetricsSettings::default(),
            ecpds: None,
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
            admin_roles: HashMap::from([("testrealm".to_string(), vec!["admin".to_string()])]),
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
            admin_roles: HashMap::from([("testrealm".to_string(), vec!["admin".to_string()])]),
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
            admin_roles: HashMap::from([("testrealm".to_string(), vec!["admin".to_string()])]),
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
            admin_roles: HashMap::from([("testrealm".to_string(), vec!["admin".to_string()])]),
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
    fn rejects_enabled_auth_with_empty_admin_role_list_for_realm() {
        let auth = AuthSettings {
            enabled: true,
            mode: AuthMode::Direct,
            auth_o_tron_url: "http://auth-o-tron:8080".to_string(),
            jwt_secret: "secret".to_string(),
            admin_roles: HashMap::from([("testrealm".to_string(), vec![])]),
            ..AuthSettings::default()
        };
        let err = validate_auth_settings(&auth).expect_err("should fail");
        assert!(err.to_string().contains("must have at least one role"));
    }

    #[test]
    fn rejects_enabled_auth_with_whitespace_admin_role() {
        let auth = AuthSettings {
            enabled: true,
            mode: AuthMode::Direct,
            auth_o_tron_url: "http://auth-o-tron:8080".to_string(),
            jwt_secret: "secret".to_string(),
            admin_roles: HashMap::from([(
                "testrealm".to_string(),
                vec!["admin".to_string(), "   ".to_string()],
            )]),
            ..AuthSettings::default()
        };
        let err = validate_auth_settings(&auth).expect_err("should fail");
        assert!(
            err.to_string()
                .contains("must not contain empty or whitespace-only role entries")
        );
    }

    #[test]
    fn rejects_enabled_auth_with_padded_admin_realm_key() {
        let auth = AuthSettings {
            enabled: true,
            mode: AuthMode::Direct,
            auth_o_tron_url: "http://auth-o-tron:8080".to_string(),
            jwt_secret: "secret".to_string(),
            admin_roles: HashMap::from([(" testrealm ".to_string(), vec!["admin".to_string()])]),
            ..AuthSettings::default()
        };
        let err = validate_auth_settings(&auth).expect_err("should fail");
        assert!(err.to_string().contains("leading or trailing whitespace"));
    }

    #[test]
    fn rejects_enabled_auth_with_padded_admin_role() {
        let auth = AuthSettings {
            enabled: true,
            mode: AuthMode::Direct,
            auth_o_tron_url: "http://auth-o-tron:8080".to_string(),
            jwt_secret: "secret".to_string(),
            admin_roles: HashMap::from([("testrealm".to_string(), vec!["admin ".to_string()])]),
            ..AuthSettings::default()
        };
        let err = validate_auth_settings(&auth).expect_err("should fail");
        assert!(err.to_string().contains("leading or trailing whitespace"));
    }

    #[test]
    fn rejects_enabled_auth_with_padded_auth_o_tron_url() {
        let auth = AuthSettings {
            enabled: true,
            mode: AuthMode::Direct,
            auth_o_tron_url: " http://auth-o-tron:8080 ".to_string(),
            jwt_secret: "secret".to_string(),
            admin_roles: HashMap::from([("testrealm".to_string(), vec!["admin".to_string()])]),
            ..AuthSettings::default()
        };
        let err = validate_auth_settings(&auth).expect_err("should fail");
        assert!(
            err.to_string()
                .contains("auth.auth_o_tron_url must not have leading or trailing whitespace")
        );
    }

    #[test]
    fn rejects_enabled_auth_with_padded_jwt_secret() {
        let auth = AuthSettings {
            enabled: true,
            mode: AuthMode::Direct,
            auth_o_tron_url: "http://auth-o-tron:8080".to_string(),
            jwt_secret: " secret ".to_string(),
            admin_roles: HashMap::from([("testrealm".to_string(), vec!["admin".to_string()])]),
            ..AuthSettings::default()
        };
        let err = validate_auth_settings(&auth).expect_err("should fail");
        assert!(
            err.to_string()
                .contains("auth.jwt_secret must not have leading or trailing whitespace")
        );
    }

    #[test]
    fn rejects_enabled_auth_with_zero_timeout() {
        let auth = AuthSettings {
            enabled: true,
            mode: AuthMode::Direct,
            auth_o_tron_url: "http://auth-o-tron:8080".to_string(),
            jwt_secret: "secret".to_string(),
            admin_roles: HashMap::from([("testrealm".to_string(), vec!["admin".to_string()])]),
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
            admin_roles: HashMap::from([("testrealm".to_string(), vec!["admin".to_string()])]),
            ..AuthSettings::default()
        };
        let err = validate_auth_settings(&auth).expect_err("should fail");
        assert!(
            err.to_string()
                .contains("auth.auth_o_tron_url must not have leading or trailing whitespace")
        );
    }

    #[test]
    fn rejects_enabled_auth_with_whitespace_jwt_secret() {
        let auth = AuthSettings {
            enabled: true,
            mode: AuthMode::Direct,
            auth_o_tron_url: "http://auth-o-tron:8080".to_string(),
            jwt_secret: "   ".to_string(),
            admin_roles: HashMap::from([("testrealm".to_string(), vec!["admin".to_string()])]),
            ..AuthSettings::default()
        };
        let err = validate_auth_settings(&auth).expect_err("should fail");
        assert!(
            err.to_string()
                .contains("auth.jwt_secret must not have leading or trailing whitespace")
        );
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
                    read_roles: None,
                    write_roles: None,
                    plugins: None,
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
    fn rejects_stream_auth_roles_when_global_auth_is_disabled() {
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
                    read_roles: Some(HashMap::from([(
                        "testrealm".to_string(),
                        vec!["reader".to_string()],
                    )])),
                    write_roles: None,
                    plugins: None,
                }),
            },
        );

        let settings = basic_settings_with_schema(schema_map);
        let err = validate_stream_auth_settings(&settings)
            .expect_err("stream auth roles should fail when auth is disabled");
        assert!(
            err.to_string()
                .contains("Schema 'diss' sets auth roles but auth is globally disabled")
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
                    read_roles: Some(HashMap::from([(
                        "testrealm".to_string(),
                        vec!["reader".to_string()],
                    )])),
                    write_roles: Some(HashMap::from([(
                        "testrealm".to_string(),
                        vec!["producer".to_string()],
                    )])),
                    plugins: None,
                }),
            },
        );

        let mut settings = basic_settings_with_schema(schema_map);
        settings.auth.enabled = true;
        settings.auth.mode = AuthMode::Direct;
        settings.auth.auth_o_tron_url = "http://auth-o-tron:8080".to_string();
        settings.auth.jwt_secret = "secret".to_string();
        settings.auth.admin_roles =
            HashMap::from([("testrealm".to_string(), vec!["admin".to_string()])]);

        validate_stream_auth_settings(&settings)
            .expect("stream auth should be accepted when auth is enabled");
    }

    #[test]
    fn accepts_stream_auth_required_without_role_lists() {
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
                    read_roles: None,
                    write_roles: None,
                    plugins: None,
                }),
            },
        );

        let mut settings = basic_settings_with_schema(schema_map);
        settings.auth.enabled = true;
        settings.auth.mode = AuthMode::Direct;
        settings.auth.auth_o_tron_url = "http://auth-o-tron:8080".to_string();
        settings.auth.jwt_secret = "secret".to_string();
        settings.auth.admin_roles =
            HashMap::from([("testrealm".to_string(), vec!["admin".to_string()])]);

        validate_stream_auth_settings(&settings)
            .expect("required=true without role lists should be accepted");
    }

    fn schema_with_plugins(
        event_type: &str,
        plugins: Option<Vec<String>>,
        auth_required: bool,
    ) -> HashMap<String, EventSchema> {
        let mut schema_map = HashMap::new();
        schema_map.insert(
            event_type.to_string(),
            EventSchema {
                payload: None,
                topic: Some(TopicConfig {
                    base: event_type.to_string(),
                    key_order: vec![],
                }),
                endpoint: None,
                identifier: HashMap::new(),
                storage_policy: None,
                auth: Some(crate::configuration::StreamAuthConfig {
                    required: auth_required,
                    read_roles: None,
                    write_roles: None,
                    plugins,
                }),
            },
        );
        schema_map
    }

    #[test]
    fn rejects_empty_plugins_list() {
        let settings = basic_settings_with_schema(schema_with_plugins("diss", Some(vec![]), true));
        let err = validate_stream_plugin_settings(&settings)
            .expect_err("empty plugins list must be rejected");
        assert!(
            err.to_string().contains("auth.plugins must not be empty"),
            "got: {err}"
        );
    }

    #[test]
    fn rejects_unknown_plugin_name() {
        let settings = basic_settings_with_schema(schema_with_plugins(
            "diss",
            Some(vec!["typo".to_string()]),
            true,
        ));
        let err = validate_stream_plugin_settings(&settings)
            .expect_err("unknown plugin name must be rejected");
        let msg = err.to_string();
        assert!(msg.contains("unknown plugin 'typo'"), "got: {msg}");
        assert!(msg.contains("Known plugins"), "got: {msg}");
    }

    #[cfg(feature = "ecpds")]
    #[test]
    fn rejects_ecpds_plugin_when_auth_required_is_false() {
        let settings = basic_settings_with_schema(schema_with_plugins(
            "diss",
            Some(vec!["ecpds".to_string()]),
            false,
        ));
        let err = validate_stream_plugin_settings(&settings).expect_err(
            "ecpds plugin paired with auth.required=false must be rejected when feature is on",
        );
        assert!(
            err.to_string().contains("auth.required is false"),
            "got: {err}"
        );
    }

    #[test]
    fn unknown_plugin_error_takes_precedence_over_required_false() {
        let settings = basic_settings_with_schema(schema_with_plugins(
            "diss",
            Some(vec!["typo".to_string()]),
            false,
        ));
        let err = validate_stream_plugin_settings(&settings)
            .expect_err("unknown-plugin error must surface ahead of required-false");
        assert!(
            err.to_string().contains("unknown plugin 'typo'"),
            "got: {err}"
        );
    }

    #[cfg(not(feature = "ecpds"))]
    #[test]
    fn rejects_ecpds_plugin_when_ecpds_feature_is_off() {
        let settings = basic_settings_with_schema(schema_with_plugins(
            "diss",
            Some(vec!["ecpds".to_string()]),
            true,
        ));
        let err = validate_stream_plugin_settings(&settings)
            .expect_err("ecpds plugin must be rejected when ecpds feature is off");
        let msg = err.to_string();
        assert!(
            msg.contains("requires the 'ecpds' Cargo feature"),
            "got: {msg}"
        );
        assert!(msg.contains("--features ecpds"), "got: {msg}");
    }

    #[cfg(feature = "ecpds")]
    #[test]
    fn accepts_ecpds_plugin_when_ecpds_feature_is_on() {
        let settings = basic_settings_with_schema(schema_with_plugins(
            "diss",
            Some(vec!["ecpds".to_string()]),
            true,
        ));
        validate_stream_plugin_settings(&settings)
            .expect("ecpds plugin with required=true must be accepted when ecpds feature is on");
    }

    #[test]
    fn accepts_no_plugins_field() {
        let settings = basic_settings_with_schema(schema_with_plugins("diss", None, true));
        validate_stream_plugin_settings(&settings)
            .expect("schemas without auth.plugins must be accepted");
    }

    #[test]
    fn rejects_stream_read_roles_with_whitespace_entry_when_auth_enabled() {
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
                    read_roles: Some(HashMap::from([(
                        "testrealm".to_string(),
                        vec!["reader".to_string(), " ".to_string()],
                    )])),
                    write_roles: None,
                    plugins: None,
                }),
            },
        );

        let mut settings = basic_settings_with_schema(schema_map);
        settings.auth.enabled = true;
        settings.auth.mode = AuthMode::Direct;
        settings.auth.auth_o_tron_url = "http://auth-o-tron:8080".to_string();
        settings.auth.jwt_secret = "secret".to_string();
        settings.auth.admin_roles =
            HashMap::from([("testrealm".to_string(), vec!["admin".to_string()])]);

        let err = validate_stream_auth_settings(&settings).expect_err("should fail");
        assert!(
            err.to_string()
                .contains("must not contain empty or whitespace-only role entries")
        );
    }

    #[test]
    fn rejects_stream_empty_read_roles_map() {
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
                    read_roles: Some(HashMap::new()),
                    write_roles: None,
                    plugins: None,
                }),
            },
        );

        let mut settings = basic_settings_with_schema(schema_map);
        settings.auth.enabled = true;
        settings.auth.mode = AuthMode::Direct;
        settings.auth.auth_o_tron_url = "http://auth-o-tron:8080".to_string();
        settings.auth.jwt_secret = "secret".to_string();
        settings.auth.admin_roles =
            HashMap::from([("testrealm".to_string(), vec!["admin".to_string()])]);

        let err = validate_stream_auth_settings(&settings).expect_err("should fail");
        assert!(
            err.to_string()
                .contains("auth.read_roles must not be an empty map")
        );
    }

    #[test]
    fn rejects_stream_empty_write_roles_map() {
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
                    read_roles: None,
                    write_roles: Some(HashMap::new()),
                    plugins: None,
                }),
            },
        );

        let mut settings = basic_settings_with_schema(schema_map);
        settings.auth.enabled = true;
        settings.auth.mode = AuthMode::Direct;
        settings.auth.auth_o_tron_url = "http://auth-o-tron:8080".to_string();
        settings.auth.jwt_secret = "secret".to_string();
        settings.auth.admin_roles =
            HashMap::from([("testrealm".to_string(), vec!["admin".to_string()])]);

        let err = validate_stream_auth_settings(&settings).expect_err("should fail");
        assert!(
            err.to_string()
                .contains("auth.write_roles must not be an empty map")
        );
    }

    #[test]
    fn rejects_stream_read_roles_with_empty_role_list_for_realm() {
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
                    read_roles: Some(HashMap::from([("testrealm".to_string(), vec![])])),
                    write_roles: None,
                    plugins: None,
                }),
            },
        );

        let mut settings = basic_settings_with_schema(schema_map);
        settings.auth.enabled = true;
        settings.auth.mode = AuthMode::Direct;
        settings.auth.auth_o_tron_url = "http://auth-o-tron:8080".to_string();
        settings.auth.jwt_secret = "secret".to_string();
        settings.auth.admin_roles =
            HashMap::from([("testrealm".to_string(), vec!["admin".to_string()])]);

        let err = validate_stream_auth_settings(&settings).expect_err("should fail");
        assert!(err.to_string().contains("must have at least one role"));
    }

    #[test]
    fn rejects_stream_read_roles_with_padded_realm_key() {
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
                    read_roles: Some(HashMap::from([(
                        " testrealm".to_string(),
                        vec!["reader".to_string()],
                    )])),
                    write_roles: None,
                    plugins: None,
                }),
            },
        );

        let mut settings = basic_settings_with_schema(schema_map);
        settings.auth.enabled = true;
        settings.auth.mode = AuthMode::Direct;
        settings.auth.auth_o_tron_url = "http://auth-o-tron:8080".to_string();
        settings.auth.jwt_secret = "secret".to_string();
        settings.auth.admin_roles =
            HashMap::from([("testrealm".to_string(), vec!["admin".to_string()])]);

        let err = validate_stream_auth_settings(&settings).expect_err("should fail");
        assert!(err.to_string().contains("leading or trailing whitespace"));
    }

    #[test]
    fn rejects_roles_when_required_is_false_and_auth_enabled() {
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
                    read_roles: Some(HashMap::from([(
                        "testrealm".to_string(),
                        vec!["reader".to_string()],
                    )])),
                    write_roles: None,
                    plugins: None,
                }),
            },
        );

        let mut settings = basic_settings_with_schema(schema_map);
        settings.auth.enabled = true;
        settings.auth.mode = AuthMode::Direct;
        settings.auth.auth_o_tron_url = "http://auth-o-tron:8080".to_string();
        settings.auth.jwt_secret = "secret".to_string();
        settings.auth.admin_roles =
            HashMap::from([("testrealm".to_string(), vec!["admin".to_string()])]);

        let err = validate_stream_auth_settings(&settings).expect_err("should fail");
        assert!(
            err.to_string()
                .contains("auth roles while auth.required=false")
        );
    }

    #[test]
    fn accepts_enabled_trusted_proxy_auth_with_required_fields() {
        let auth = AuthSettings {
            enabled: true,
            mode: AuthMode::TrustedProxy,
            jwt_secret: "secret".to_string(),
            admin_roles: HashMap::from([("testrealm".to_string(), vec!["admin".to_string()])]),
            ..AuthSettings::default()
        };
        assert!(validate_auth_settings(&auth).is_ok());
    }

    #[test]
    fn rejects_enabled_trusted_proxy_auth_without_jwt_secret() {
        let auth = AuthSettings {
            enabled: true,
            mode: AuthMode::TrustedProxy,
            admin_roles: HashMap::from([("testrealm".to_string(), vec!["admin".to_string()])]),
            ..AuthSettings::default()
        };
        let err = validate_auth_settings(&auth).expect_err("should fail");
        assert!(
            err.to_string()
                .contains("auth.mode=trusted_proxy requires auth.jwt_secret")
        );
    }

    fn settings_with_metrics(app_port: u16, metrics: MetricsSettings) -> Settings {
        Settings {
            application: ApplicationSettings {
                host: "127.0.0.1".to_string(),
                port: app_port,
                base_url: "http://localhost".to_string(),
                static_files_path: "/tmp".to_string(),
            },
            notification_backend: NotificationBackendSettings {
                kind: "in_memory".to_string(),
                in_memory: None,
                jetstream: None,
            },
            logging: None,
            notification_schema: None,
            watch_endpoint: WatchEndpointSettings::default(),
            auth: AuthSettings::default(),
            metrics,
            ecpds: None,
        }
    }

    #[test]
    fn accepts_disabled_metrics() {
        let settings = settings_with_metrics(8000, MetricsSettings::default());
        validate_metrics_settings(&settings).expect("disabled metrics should pass");
    }

    #[test]
    fn rejects_enabled_metrics_without_port() {
        let settings = settings_with_metrics(
            8000,
            MetricsSettings {
                enabled: true,
                port: None,
                ..Default::default()
            },
        );
        let err = validate_metrics_settings(&settings).expect_err("should fail");
        assert!(
            err.to_string()
                .contains("metrics.port is required when metrics.enabled=true")
        );
    }

    #[test]
    fn rejects_metrics_port_zero() {
        let settings = settings_with_metrics(
            8000,
            MetricsSettings {
                enabled: true,
                port: Some(0),
                ..Default::default()
            },
        );
        let err = validate_metrics_settings(&settings).expect_err("should fail");
        assert!(err.to_string().contains("metrics.port must not be 0"));
    }

    #[test]
    fn rejects_metrics_port_equal_to_application_port() {
        let settings = settings_with_metrics(
            8000,
            MetricsSettings {
                enabled: true,
                port: Some(8000),
                ..Default::default()
            },
        );
        let err = validate_metrics_settings(&settings).expect_err("should fail");
        assert!(
            err.to_string()
                .contains("metrics.port must differ from application.port")
        );
    }

    #[test]
    fn accepts_enabled_metrics_with_distinct_port() {
        let settings = settings_with_metrics(
            8000,
            MetricsSettings {
                enabled: true,
                port: Some(9090),
                ..Default::default()
            },
        );
        validate_metrics_settings(&settings).expect("distinct port should pass");
    }

    #[cfg(feature = "ecpds")]
    mod ecpds {
        use super::super::validate_ecpds_settings;
        use super::basic_settings_with_schema;
        use crate::configuration::{
            EventSchema, IdentifierFieldConfig, Settings, StreamAuthConfig, TopicConfig,
        };
        use aviso_validators::ValidationRules;
        use std::collections::HashMap;

        fn ecpds_protected_schema(
            match_key: &str,
            match_required: bool,
        ) -> HashMap<String, EventSchema> {
            let mut identifier = HashMap::new();
            identifier.insert(
                match_key.to_string(),
                IdentifierFieldConfig::with_rule(ValidationRules::StringHandler {
                    required: match_required,
                    max_length: None,
                }),
            );
            let mut schema = HashMap::new();
            schema.insert(
                "diss".to_string(),
                EventSchema {
                    payload: None,
                    topic: Some(TopicConfig {
                        base: "diss".to_string(),
                        key_order: vec![match_key.to_string()],
                    }),
                    endpoint: None,
                    identifier,
                    storage_policy: None,
                    auth: Some(StreamAuthConfig {
                        required: true,
                        read_roles: None,
                        write_roles: None,
                        plugins: Some(vec!["ecpds".to_string()]),
                    }),
                },
            );
            schema
        }

        fn settings_with_ecpds(
            ecpds: aviso_ecpds::config::EcpdsConfig,
            match_key: &str,
            match_required: bool,
        ) -> Settings {
            let mut s =
                basic_settings_with_schema(ecpds_protected_schema(match_key, match_required));
            s.ecpds = Some(ecpds);
            s
        }

        fn good_ecpds_config() -> aviso_ecpds::config::EcpdsConfig {
            aviso_ecpds::config::EcpdsConfig {
                username: "u".to_string(),
                password: "p".to_string(),
                target_field: "name".to_string(),
                match_key: "destination".to_string(),
                cache_ttl_seconds: 300,
                max_entries: 10_000,
                request_timeout_seconds: 30,
                connect_timeout_seconds: 5,
                partial_outage_policy: aviso_ecpds::config::PartialOutagePolicy::Strict,
                servers: vec!["http://localhost".to_string()],
            }
        }

        #[test]
        fn accepts_well_formed_settings() {
            let settings = settings_with_ecpds(good_ecpds_config(), "destination", true);
            validate_ecpds_settings(&settings).expect("should accept");
        }

        #[test]
        fn rejects_when_plugin_referenced_but_no_ecpds_section() {
            let mut s = basic_settings_with_schema(ecpds_protected_schema("destination", true));
            s.ecpds = None;
            let err = validate_ecpds_settings(&s).expect_err("must fail");
            assert!(
                err.to_string().contains("no 'ecpds' configuration section"),
                "got: {err}"
            );
        }

        #[test]
        fn rejects_invalid_server_url() {
            let mut cfg = good_ecpds_config();
            cfg.servers = vec!["not a url".to_string()];
            let err = validate_ecpds_settings(&settings_with_ecpds(cfg, "destination", true))
                .expect_err("must fail");
            assert!(err.to_string().contains("not a valid URL"), "got: {err}");
        }

        #[test]
        fn rejects_unsupported_scheme() {
            let mut cfg = good_ecpds_config();
            cfg.servers = vec!["ftp://example.com".to_string()];
            let err = validate_ecpds_settings(&settings_with_ecpds(cfg, "destination", true))
                .expect_err("must fail");
            assert!(
                err.to_string().contains("unsupported scheme 'ftp'"),
                "got: {err}"
            );
        }

        #[test]
        fn rejects_server_url_with_query_string() {
            let mut cfg = good_ecpds_config();
            cfg.servers = vec!["http://example.com/?already=set".to_string()];
            let err = validate_ecpds_settings(&settings_with_ecpds(cfg, "destination", true))
                .expect_err("must fail");
            assert!(
                err.to_string().contains("must not contain a query string"),
                "got: {err}"
            );
        }

        #[test]
        fn rejects_server_url_with_fragment() {
            let mut cfg = good_ecpds_config();
            cfg.servers = vec!["http://example.com/#frag".to_string()];
            let err = validate_ecpds_settings(&settings_with_ecpds(cfg, "destination", true))
                .expect_err("must fail");
            assert!(
                err.to_string().contains("must not contain a URL fragment"),
                "got: {err}"
            );
        }

        #[test]
        fn accepts_https_and_http_schemes() {
            for url in ["http://a.example", "https://b.example"] {
                let mut cfg = good_ecpds_config();
                cfg.servers = vec![url.to_string()];
                validate_ecpds_settings(&settings_with_ecpds(cfg, "destination", true))
                    .unwrap_or_else(|e| panic!("should accept {url}: {e}"));
            }
        }

        #[test]
        fn rejects_zero_cache_ttl() {
            let mut cfg = good_ecpds_config();
            cfg.cache_ttl_seconds = 0;
            let err = validate_ecpds_settings(&settings_with_ecpds(cfg, "destination", true))
                .expect_err("must fail");
            assert!(
                err.to_string()
                    .contains("cache_ttl_seconds must be greater than zero"),
                "got: {err}"
            );
        }

        #[test]
        fn rejects_zero_max_entries() {
            let mut cfg = good_ecpds_config();
            cfg.max_entries = 0;
            let err = validate_ecpds_settings(&settings_with_ecpds(cfg, "destination", true))
                .expect_err("must fail");
            assert!(
                err.to_string()
                    .contains("max_entries must be greater than zero"),
                "got: {err}"
            );
        }

        #[test]
        fn rejects_zero_request_timeout() {
            let mut cfg = good_ecpds_config();
            cfg.request_timeout_seconds = 0;
            let err = validate_ecpds_settings(&settings_with_ecpds(cfg, "destination", true))
                .expect_err("must fail");
            assert!(
                err.to_string()
                    .contains("request_timeout_seconds must be greater than zero"),
                "got: {err}"
            );
        }

        #[test]
        fn rejects_zero_connect_timeout() {
            let mut cfg = good_ecpds_config();
            cfg.connect_timeout_seconds = 0;
            let err = validate_ecpds_settings(&settings_with_ecpds(cfg, "destination", true))
                .expect_err("must fail");
            assert!(
                err.to_string()
                    .contains("connect_timeout_seconds must be greater than zero"),
                "got: {err}"
            );
        }

        #[test]
        fn rejects_empty_target_field() {
            let mut cfg = good_ecpds_config();
            cfg.target_field = String::new();
            let err = validate_ecpds_settings(&settings_with_ecpds(cfg, "destination", true))
                .expect_err("must fail");
            assert!(
                err.to_string().contains("target_field must not be empty"),
                "got: {err}"
            );
        }

        #[test]
        fn rejects_match_key_with_whitespace() {
            let mut cfg = good_ecpds_config();
            cfg.match_key = "dest ination".to_string();
            let err = validate_ecpds_settings(&settings_with_ecpds(cfg, "destination", true))
                .expect_err("must fail");
            assert!(
                err.to_string()
                    .contains("must be a single bare identifier name"),
                "got: {err}"
            );
        }

        #[test]
        fn rejects_match_key_not_in_key_order() {
            let cfg = good_ecpds_config();
            let err = validate_ecpds_settings(&settings_with_ecpds(cfg, "other_key", true))
                .expect_err("must fail");
            assert!(
                err.to_string().contains("not found in key_order"),
                "got: {err}"
            );
        }

        #[test]
        fn rejects_match_key_not_required_in_schema() {
            let cfg = good_ecpds_config();
            let err = validate_ecpds_settings(&settings_with_ecpds(cfg, "destination", false))
                .expect_err("must fail");
            assert!(
                err.to_string().contains("must be required: true in schema"),
                "got: {err}"
            );
        }
    }
}
