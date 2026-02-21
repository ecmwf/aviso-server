use crate::configuration::{
    EventStoragePolicy, JetStreamDiscardPolicy, JetStreamRetentionPolicy, JetStreamStorageType,
    Settings, parse_retention_time_spec, parse_size_spec,
};
use crate::notification::topic_parser::derive_event_type_from_topic;
use crate::notification_backend::jetstream::backend::JetStreamBackend;
use crate::telemetry::{SERVICE_NAME, SERVICE_VERSION};
use anyhow::{Context, Result, bail};
use async_nats::jetstream::stream::{
    Compression, Config as StreamConfig, DiscardPolicy, RetentionPolicy, StorageType,
};
use tracing::{debug, info, warn};

/// Ensure a stream exists for the given topic
/// Creates streams on-demand based on topic base (e.g., "diss.foo.bar" -> "DISS" stream)
/// This prevents subject overlap by creating separate streams for each base
///
/// # Arguments
/// * `topic` - Full topic name (e.g., "diss.FOO.E1.od.g.1.20190810.0.enfo.1")
///
/// # Returns
/// * `Result<String>` - Stream name that handles this topic or error if creation fails
pub async fn ensure_stream_for_topic(backend: &JetStreamBackend, topic: &str) -> Result<String> {
    // Extract base from topic (first part before '.')
    let base =
        derive_event_type_from_topic(topic).context("Failed to extract event type from topic")?;

    // Create stream name by uppercasing the base
    let stream_name = base.to_uppercase();
    // Create subject pattern to match all topics with this base
    let subject_pattern = format!("{}.>", base);

    debug!(
        topic = %topic,
        base = %base,
        stream_name = %stream_name,
        subject_pattern = %subject_pattern,
        "Determining stream for topic"
    );

    // Build desired stream configuration for this base.
    let (stream_config, schema_policy_owner) =
        build_stream_config_for_base(backend, &stream_name, &subject_pattern)?;
    let max_age_seconds = stream_config.max_age.as_secs();
    let schema_policy_applied = schema_policy_owner.is_some();
    let schema_policy_owner_field = schema_policy_owner.as_deref().unwrap_or("");

    // If stream exists, reconcile mutable settings.
    // If lookup fails, try create path as a fallback (JetStream metadata lookups can be
    // temporarily unavailable while publish/create still succeeds).
    match backend.jetstream.get_stream(&stream_name).await {
        Ok(stream) => {
            debug!(
                stream_name = %stream_name,
                subject_pattern = %subject_pattern,
                "Stream already exists; checking configuration drift"
            );
            return reconcile_existing_stream_config(
                backend,
                stream,
                &stream_name,
                &subject_pattern,
                &stream_config,
                schema_policy_applied,
                schema_policy_owner_field,
            )
            .await;
        }
        Err(error) if is_stream_not_found_error(&error) => {
            debug!(
                stream_name = %stream_name,
                subject_pattern = %subject_pattern,
                error = %error,
                "Stream lookup returned not-found; proceeding with create"
            );
        }
        Err(error) => {
            warn!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_domain = "backend",
                event_name = "backend.jetstream.stream.lookup.failed_fallback_create",
                stream_name = %stream_name,
                subject_pattern = %subject_pattern,
                error = %error,
                "Failed to lookup stream; falling back to create path"
            );
        }
    }

    info!(
        service_name = SERVICE_NAME,
        service_version = SERVICE_VERSION,
        event_domain = "backend",
        event_name = "backend.jetstream.stream.create.started",
        stream_name = %stream_name,
        subject_pattern = %subject_pattern,
        "Creating new stream for base topic"
    );

    info!(
        service_name = SERVICE_NAME,
        service_version = SERVICE_VERSION,
        event_domain = "backend",
        event_name = "backend.jetstream.stream.create.config",
        stream_name = %stream_name,
        subject_pattern = %subject_pattern,
        max_messages = stream_config.max_messages,
        max_bytes = stream_config.max_bytes,
        max_age_seconds = max_age_seconds,
        max_messages_per_subject = stream_config.max_messages_per_subject,
        replicas = stream_config.num_replicas,
        compression = ?stream_config.compression,
        schema_policy_applied = schema_policy_applied,
        schema_policy_owner = schema_policy_owner_field,
        "Applying effective stream configuration"
    );

    let max_messages = stream_config.max_messages;
    let max_bytes = stream_config.max_bytes;
    let max_messages_per_subject = stream_config.max_messages_per_subject;
    let replicas = stream_config.num_replicas;
    let compression = stream_config.compression.clone();

    // Attempt to create the stream with proper error handling
    match backend.jetstream.create_stream(stream_config).await {
        Ok(_) => {
            info!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_domain = "backend",
                event_name = "backend.jetstream.stream.create.succeeded",
                stream_name = %stream_name,
                subject_pattern = %subject_pattern,
                max_messages = max_messages,
                max_bytes = max_bytes,
                max_age_seconds = max_age_seconds,
                max_messages_per_subject = max_messages_per_subject,
                replicas = replicas,
                compression = ?compression,
                schema_policy_applied = schema_policy_applied,
                schema_policy_owner = schema_policy_owner_field,
                "Stream created successfully"
            );
            Ok(stream_name)
        }
        Err(e) => {
            let error_msg = e.to_string();
            // Handle race condition where another replica creates the stream
            if error_msg.contains("stream name already in use") {
                info!(
                    service_name = SERVICE_NAME,
                    service_version = SERVICE_VERSION,
                    event_domain = "backend",
                    event_name = "backend.jetstream.stream.create.race_won_by_peer",
                    stream_name = %stream_name,
                    "Stream created by another replica"
                );
                Ok(stream_name)
            } else {
                warn!(
                    service_name = SERVICE_NAME,
                    service_version = SERVICE_VERSION,
                    event_domain = "backend",
                    event_name = "backend.jetstream.stream.create.failed",
                    stream_name = %stream_name,
                    subject_pattern = %subject_pattern,
                    max_messages = max_messages,
                    max_bytes = max_bytes,
                    max_age_seconds = max_age_seconds,
                    max_messages_per_subject = max_messages_per_subject,
                    replicas = replicas,
                    compression = ?compression,
                    schema_policy_applied = schema_policy_applied,
                    schema_policy_owner = schema_policy_owner_field,
                    error = %e,
                    "Failed to create stream"
                );
                Err(e.into())
            }
        }
    }
}

fn is_stream_not_found_error(error: &async_nats::jetstream::context::GetStreamError) -> bool {
    let result = matches!(
        error.kind(),
        async_nats::jetstream::context::GetStreamErrorKind::JetStream(js_error)
            if js_error.error_code() == async_nats::jetstream::ErrorCode::STREAM_NOT_FOUND
                // Some server/client combinations may not populate JetStream-specific
                // err_code for stream-not-found. Fall back to HTTP status code.
                || js_error.code() == 404
    );

    if !result {
        debug!(
            error = %error,
            "get_stream error was not classified as stream-not-found"
        );
    }

    result
}

async fn reconcile_existing_stream_config(
    backend: &JetStreamBackend,
    stream: async_nats::jetstream::stream::Stream,
    stream_name: &str,
    subject_pattern: &str,
    desired_config: &StreamConfig,
    schema_policy_applied: bool,
    schema_policy_owner: &str,
) -> Result<String> {
    let current_config = &stream.cached_info().config;

    let (update_config, changes) = merged_reconciled_config(current_config, desired_config);
    if changes.is_empty() {
        debug!(
            service_name = SERVICE_NAME,
            service_version = SERVICE_VERSION,
            event_domain = "backend",
            event_name = "backend.jetstream.stream.reconcile.noop",
            stream_name = %stream_name,
            subject_pattern = %subject_pattern,
            "Existing stream already matches desired mutable configuration"
        );
        return Ok(stream_name.to_string());
    }

    info!(
        service_name = SERVICE_NAME,
        service_version = SERVICE_VERSION,
        event_domain = "backend",
        event_name = "backend.jetstream.stream.reconcile.started",
        stream_name = %stream_name,
        subject_pattern = %subject_pattern,
        changed_fields = ?changes,
        schema_policy_applied = schema_policy_applied,
        schema_policy_owner = schema_policy_owner,
        "Reconciling existing stream mutable configuration"
    );

    if let Err(error) = backend.jetstream.update_stream(update_config).await {
        warn!(
            service_name = SERVICE_NAME,
            service_version = SERVICE_VERSION,
            event_domain = "backend",
            event_name = "backend.jetstream.stream.reconcile.failed",
            stream_name = %stream_name,
            subject_pattern = %subject_pattern,
            changed_fields = ?changes,
            schema_policy_applied = schema_policy_applied,
            schema_policy_owner = schema_policy_owner,
            error = %error,
            "Stream reconciliation failed; continuing with existing stream configuration"
        );
        return Ok(stream_name.to_string());
    }

    info!(
        service_name = SERVICE_NAME,
        service_version = SERVICE_VERSION,
        event_domain = "backend",
        event_name = "backend.jetstream.stream.reconcile.succeeded",
        stream_name = %stream_name,
        subject_pattern = %subject_pattern,
        changed_fields = ?changes,
        schema_policy_applied = schema_policy_applied,
        schema_policy_owner = schema_policy_owner,
        "Stream mutable configuration reconciled"
    );

    Ok(stream_name.to_string())
}

fn merged_reconciled_config(
    current: &StreamConfig,
    desired: &StreamConfig,
) -> (StreamConfig, Vec<&'static str>) {
    let mut merged = current.clone();
    let mut changed_fields = Vec::new();

    // Reconcile stream settings that Aviso owns for managed streams.
    if merged.subjects != desired.subjects {
        merged.subjects = desired.subjects.clone();
        changed_fields.push("subjects");
    }
    if merged.max_messages != desired.max_messages {
        merged.max_messages = desired.max_messages;
        changed_fields.push("max_messages");
    }
    if merged.max_bytes != desired.max_bytes {
        merged.max_bytes = desired.max_bytes;
        changed_fields.push("max_bytes");
    }
    if merged.max_age != desired.max_age {
        merged.max_age = desired.max_age;
        changed_fields.push("max_age");
    }
    if merged.max_messages_per_subject != desired.max_messages_per_subject {
        merged.max_messages_per_subject = desired.max_messages_per_subject;
        changed_fields.push("max_messages_per_subject");
    }
    if merged.num_replicas != desired.num_replicas {
        merged.num_replicas = desired.num_replicas;
        changed_fields.push("num_replicas");
    }
    if merged.compression != desired.compression {
        merged.compression = desired.compression.clone();
        changed_fields.push("compression");
    }
    if merged.discard != desired.discard {
        merged.discard = desired.discard;
        changed_fields.push("discard");
    }
    if merged.retention != desired.retention {
        merged.retention = desired.retention;
        changed_fields.push("retention");
    }

    (merged, changed_fields)
}

fn build_stream_config_for_base(
    backend: &JetStreamBackend,
    stream_name: &str,
    subject_pattern: &str,
) -> Result<(StreamConfig, Option<String>)> {
    let storage_type = match backend.config.storage_type {
        JetStreamStorageType::File => StorageType::File,
        JetStreamStorageType::Memory => StorageType::Memory,
    };

    let retention = match backend.config.retention_policy {
        JetStreamRetentionPolicy::Limits => RetentionPolicy::Limits,
        JetStreamRetentionPolicy::Interest => RetentionPolicy::Interest,
        JetStreamRetentionPolicy::Workqueue => RetentionPolicy::WorkQueue,
    };

    let discard = match backend.config.discard_policy {
        JetStreamDiscardPolicy::Old => DiscardPolicy::Old,
        JetStreamDiscardPolicy::New => DiscardPolicy::New,
    };

    // Keep current default behavior: one message per subject unless schema policy overrides it.
    let mut config = StreamConfig {
        name: stream_name.to_string(),
        subjects: vec![subject_pattern.to_string()], // Only match this base's topics
        storage: storage_type,
        retention,
        discard,
        max_messages_per_subject: 1, // Keep only the latest message per subject
        ..Default::default()
    };

    apply_backend_defaults(&mut config, backend);

    let mut schema_policy_owner = None;
    if let Some((owner, policy)) = resolve_storage_policy_for_base(stream_name)? {
        schema_policy_owner = Some(owner);
        apply_storage_policy_overrides(&mut config, &policy)?;
    }

    debug!(
        stream_name = %stream_name,
        subject_pattern = %subject_pattern,
        storage = ?config.storage,
        retention = ?config.retention,
        max_messages = config.max_messages,
        max_bytes = config.max_bytes,
        max_age_seconds = config.max_age.as_secs(),
        replicas = config.num_replicas,
        compression = ?config.compression,
        max_messages_per_subject = config.max_messages_per_subject,
        schema_policy_applied = schema_policy_owner.is_some(),
        schema_policy_owner = ?schema_policy_owner,
        "Built effective stream configuration"
    );

    Ok((config, schema_policy_owner))
}

fn apply_backend_defaults(config: &mut StreamConfig, backend: &JetStreamBackend) {
    if let Some(max_messages) = backend.config.max_messages {
        config.max_messages = max_messages;
    }
    if let Some(max_bytes) = backend.config.max_bytes {
        config.max_bytes = max_bytes;
    }
    if let Some(retention_time) = backend.config.retention_time {
        config.max_age = retention_time;
    }
    if let Some(replicas) = backend.config.replicas {
        config.num_replicas = replicas;
    }
}

fn apply_storage_policy_overrides(
    config: &mut StreamConfig,
    policy: &EventStoragePolicy,
) -> Result<()> {
    if let Some(retention_time) = policy.retention_time.as_deref() {
        let parsed_retention = parse_retention_time_spec(retention_time)
            .map_err(|e| anyhow::anyhow!("Invalid storage_policy.retention_time: {e}"))?;
        // Guard against bypassed startup validation paths.
        if parsed_retention.is_zero() {
            bail!("Invalid storage_policy.retention_time: value must be > 0");
        }
        config.max_age = parsed_retention;
    }
    if let Some(max_messages) = policy.max_messages {
        config.max_messages = max_messages;
    }
    if let Some(max_size) = policy.max_size.as_deref() {
        config.max_bytes = parse_size_spec(max_size)
            .map_err(|e| anyhow::anyhow!("Invalid storage_policy.max_size: {e}"))?;
    }
    if let Some(allow_duplicates) = policy.allow_duplicates {
        config.max_messages_per_subject = if allow_duplicates { -1 } else { 1 };
    }
    if let Some(compression_enabled) = policy.compression {
        config.compression = Some(if compression_enabled {
            Compression::S2
        } else {
            Compression::None
        });
    }
    Ok(())
}

fn resolve_storage_policy_for_base(
    stream_name: &str,
) -> Result<Option<(String, EventStoragePolicy)>> {
    let base = stream_name.to_ascii_lowercase();
    let schema = Settings::get_global_notification_schema();
    let Some(schema_map) = schema.as_ref() else {
        return Ok(None);
    };

    let mut selected: Option<(String, EventStoragePolicy)> = None;

    for (event_type, event_schema) in schema_map {
        let Some(topic_cfg) = event_schema.topic.as_ref() else {
            continue;
        };
        if topic_cfg.base != base {
            continue;
        }
        let Some(policy) = event_schema.storage_policy.clone() else {
            continue;
        };
        // Startup validation already rejects duplicate policy owners for one base.
        // Keep this guard as a defensive fallback for unexpected config lifecycle issues.
        if let Some((previous_event_type, _)) = &selected {
            bail!(
                "Multiple schemas define storage_policy for stream base '{base}': '{previous_event_type}' and '{event_type}'"
            );
        }
        selected = Some((event_type.clone(), policy));
    }

    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::{apply_storage_policy_overrides, merged_reconciled_config};
    use crate::configuration::{
        ApplicationSettings, EventSchema, EventStoragePolicy, NotificationBackendSettings,
        Settings, TopicConfig, WatchEndpointSettings,
    };
    use crate::notification_backend::jetstream::{
        backend::JetStreamBackend,
        config::JetStreamConfig,
        connection::{connect, shutdown},
    };
    use async_nats::jetstream::stream::{Compression, Config as StreamConfig};
    use std::collections::HashMap;
    use std::time::Duration;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn should_run_nats_tests() -> bool {
        std::env::var("AVISO_RUN_NATS_TESTS")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    }

    fn init_test_global_config() {
        let mut notification_schema = HashMap::new();
        notification_schema.insert(
            "reconcile_compression".to_string(),
            EventSchema {
                payload: None,
                topic: Some(TopicConfig {
                    base: "reconcile_compression".to_string(),
                    key_order: vec![],
                }),
                endpoint: None,
                identifier: HashMap::new(),
                storage_policy: Some(EventStoragePolicy {
                    retention_time: None,
                    max_messages: None,
                    max_size: None,
                    allow_duplicates: None,
                    compression: Some(true),
                }),
            },
        );

        let settings = Settings {
            application: ApplicationSettings {
                host: "127.0.0.1".to_string(),
                port: 8000,
                base_url: "http://localhost".to_string(),
                static_files_path: "./src/static".to_string(),
            },
            notification_backend: NotificationBackendSettings {
                kind: "jetstream".to_string(),
                in_memory: None,
                jetstream: None,
            },
            logging: None,
            notification_schema: Some(notification_schema),
            watch_endpoint: WatchEndpointSettings::default(),
        };
        // Global config is OnceLock-backed; repeated calls are no-op after first set.
        settings.init_global_config();
    }

    async fn connect_or_skip(config: JetStreamConfig, context: &str) -> Option<JetStreamBackend> {
        match connect(config).await {
            Ok(backend) => Some(backend),
            Err(error) => {
                eprintln!(
                    "skipping JetStream integration test ({context}): failed to connect to NATS: {error:#}"
                );
                None
            }
        }
    }

    #[test]
    fn storage_policy_overrides_limits_and_duration() {
        let mut config = StreamConfig {
            max_messages: 100,
            max_bytes: 1024,
            max_age: Duration::from_secs(3600),
            max_messages_per_subject: 1,
            ..Default::default()
        };
        let policy = EventStoragePolicy {
            retention_time: Some("2h".to_string()),
            max_messages: Some(250),
            max_size: Some("2Mi".to_string()),
            allow_duplicates: Some(true),
            compression: Some(true),
        };

        apply_storage_policy_overrides(&mut config, &policy).expect("policy should apply");

        assert_eq!(config.max_age, Duration::from_secs(7200));
        assert_eq!(config.max_messages, 250);
        assert_eq!(config.max_bytes, 2_097_152);
        assert_eq!(config.max_messages_per_subject, -1);
        assert_eq!(config.compression, Some(Compression::S2));
    }

    #[test]
    fn storage_policy_can_disable_compression_and_duplicates() {
        let mut config = StreamConfig {
            max_messages_per_subject: -1,
            ..Default::default()
        };
        let policy = EventStoragePolicy {
            allow_duplicates: Some(false),
            compression: Some(false),
            ..EventStoragePolicy::default()
        };

        apply_storage_policy_overrides(&mut config, &policy).expect("policy should apply");
        assert_eq!(config.max_messages_per_subject, 1);
        assert_eq!(config.compression, Some(Compression::None));
    }

    #[test]
    fn storage_policy_rejects_zero_retention_time() {
        let mut config = StreamConfig::default();
        let policy = EventStoragePolicy {
            retention_time: Some("0s".to_string()),
            ..EventStoragePolicy::default()
        };

        let err = apply_storage_policy_overrides(&mut config, &policy)
            .expect_err("zero retention must be rejected");
        assert!(
            err.to_string()
                .contains("Invalid storage_policy.retention_time: value must be > 0")
        );
    }

    #[test]
    fn merged_reconciled_config_updates_only_mutable_policy_fields() {
        let current = StreamConfig {
            name: "DISS".to_string(),
            subjects: vec!["diss.>".to_string()],
            max_messages: 100,
            max_bytes: 1024,
            max_age: Duration::from_secs(3600),
            max_messages_per_subject: 1,
            num_replicas: 1,
            compression: Some(Compression::None),
            ..Default::default()
        };

        let desired = StreamConfig {
            name: "DISS".to_string(),
            subjects: vec!["diss.>".to_string()],
            max_messages: 200,
            max_bytes: 2048,
            max_age: Duration::from_secs(7200),
            max_messages_per_subject: -1,
            num_replicas: 3,
            compression: Some(Compression::S2),
            ..Default::default()
        };

        let (merged, changed) = merged_reconciled_config(&current, &desired);
        assert_eq!(
            changed,
            vec![
                "max_messages",
                "max_bytes",
                "max_age",
                "max_messages_per_subject",
                "num_replicas",
                "compression"
            ]
        );
        assert_eq!(merged.max_messages, 200);
        assert_eq!(merged.max_bytes, 2048);
        assert_eq!(merged.max_age, Duration::from_secs(7200));
        assert_eq!(merged.max_messages_per_subject, -1);
        assert_eq!(merged.num_replicas, 3);
        assert_eq!(merged.compression, Some(Compression::S2));
        // Non-reconciled topology fields are preserved from current config.
        assert_eq!(merged.name, "DISS");
        assert_eq!(merged.subjects, vec!["diss.>".to_string()]);
    }

    #[test]
    fn merged_reconciled_config_noop_when_already_matching() {
        let current = StreamConfig {
            name: "MARS".to_string(),
            subjects: vec!["mars.>".to_string()],
            max_messages: 100,
            max_bytes: 1024,
            max_age: Duration::from_secs(3600),
            max_messages_per_subject: 1,
            num_replicas: 1,
            compression: Some(Compression::None),
            ..Default::default()
        };
        let desired = current.clone();

        let (merged, changed) = merged_reconciled_config(&current, &desired);
        assert!(changed.is_empty());
        assert_eq!(merged.max_messages, current.max_messages);
        assert_eq!(merged.max_bytes, current.max_bytes);
        assert_eq!(merged.max_age, current.max_age);
    }

    #[tokio::test]
    async fn existing_stream_retention_is_reconciled_when_backend_default_changes() {
        if !should_run_nats_tests() {
            return;
        }

        init_test_global_config();

        let nats_url =
            std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_string());
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let base = format!("reconcile_retention_{}", suffix);
        let topic = format!("{base}.subject");
        let stream_name = base.to_uppercase();

        let Some(backend_v1) = connect_or_skip(
            JetStreamConfig {
                nats_url: nats_url.clone(),
                timeout_seconds: 10,
                retry_attempts: 3,
                token: None,
                max_messages: None,
                max_bytes: None,
                retention_time: Some(Duration::from_secs(24 * 3600)),
                storage_type: crate::configuration::JetStreamStorageType::File,
                replicas: Some(1),
                retention_policy: crate::configuration::JetStreamRetentionPolicy::Limits,
                discard_policy: crate::configuration::JetStreamDiscardPolicy::Old,
                enable_auto_reconnect: true,
                max_reconnect_attempts: 5,
                reconnect_delay_ms: 200,
                publish_retry_attempts: 5,
                publish_retry_base_delay_ms: 150,
            },
            "retention reconcile (initial backend)",
        )
        .await
        else {
            return;
        };

        super::ensure_stream_for_topic(&backend_v1, &topic)
            .await
            .expect("stream creation should succeed");

        let stream_v1 = backend_v1
            .jetstream
            .get_stream(&stream_name)
            .await
            .expect("stream should exist after create");
        assert_eq!(stream_v1.cached_info().config.max_age.as_secs(), 24 * 3600);

        let Some(backend_v2) = connect_or_skip(
            JetStreamConfig {
                nats_url,
                timeout_seconds: 10,
                retry_attempts: 3,
                token: None,
                max_messages: None,
                max_bytes: None,
                retention_time: Some(Duration::from_secs(2 * 24 * 3600)),
                storage_type: crate::configuration::JetStreamStorageType::File,
                replicas: Some(1),
                retention_policy: crate::configuration::JetStreamRetentionPolicy::Limits,
                discard_policy: crate::configuration::JetStreamDiscardPolicy::Old,
                enable_auto_reconnect: true,
                max_reconnect_attempts: 5,
                reconnect_delay_ms: 200,
                publish_retry_attempts: 5,
                publish_retry_base_delay_ms: 150,
            },
            "retention reconcile (updated backend)",
        )
        .await
        else {
            let _ = shutdown(&backend_v1).await;
            return;
        };

        super::ensure_stream_for_topic(&backend_v2, &topic)
            .await
            .expect("reconciliation should succeed");

        let stream_v2 = backend_v2
            .jetstream
            .get_stream(&stream_name)
            .await
            .expect("stream should still exist");
        assert_eq!(
            stream_v2.cached_info().config.max_age.as_secs(),
            2 * 24 * 3600
        );

        let _ = backend_v2.jetstream.delete_stream(&stream_name).await;
        let _ = shutdown(&backend_v2).await;
        let _ = shutdown(&backend_v1).await;
    }

    #[tokio::test]
    async fn existing_stream_compression_is_reconciled_from_none_to_s2() {
        if !should_run_nats_tests() {
            return;
        }

        init_test_global_config();

        let nats_url =
            std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_string());
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let base = "reconcile_compression";
        let topic = format!("{base}.subject_{suffix}");
        let stream_name = base.to_uppercase();

        let Some(backend) = connect_or_skip(
            JetStreamConfig {
                nats_url,
                timeout_seconds: 10,
                retry_attempts: 3,
                token: None,
                max_messages: None,
                max_bytes: None,
                retention_time: None,
                storage_type: crate::configuration::JetStreamStorageType::File,
                replicas: Some(1),
                retention_policy: crate::configuration::JetStreamRetentionPolicy::Limits,
                discard_policy: crate::configuration::JetStreamDiscardPolicy::Old,
                enable_auto_reconnect: true,
                max_reconnect_attempts: 5,
                reconnect_delay_ms: 200,
                publish_retry_attempts: 5,
                publish_retry_base_delay_ms: 150,
            },
            "compression reconcile",
        )
        .await
        else {
            return;
        };

        // Create stream with compression disabled to verify reconciliation path upgrades it.
        let _ = backend.jetstream.delete_stream(&stream_name).await;
        backend
            .jetstream
            .create_stream(StreamConfig {
                name: stream_name.clone(),
                subjects: vec![format!("{base}.>")],
                compression: Some(Compression::None),
                ..Default::default()
            })
            .await
            .expect("precreate stream should succeed");

        let before = backend
            .jetstream
            .get_stream(&stream_name)
            .await
            .expect("stream should exist before reconcile");
        assert_eq!(
            before.cached_info().config.compression,
            Some(Compression::None)
        );

        super::ensure_stream_for_topic(&backend, &topic)
            .await
            .expect("reconciliation should succeed");

        let after = backend
            .jetstream
            .get_stream(&stream_name)
            .await
            .expect("stream should exist after reconcile");
        assert_eq!(
            after.cached_info().config.compression,
            Some(Compression::S2)
        );

        let _ = backend.jetstream.delete_stream(&stream_name).await;
        let _ = shutdown(&backend).await;
    }
}
