use crate::configuration::{
    JetStreamDiscardPolicy, JetStreamRetentionPolicy, JetStreamStorageType,
};
use crate::notification::topic_parser::derive_event_type_from_topic;
use crate::notification_backend::jetstream::backend::JetStreamBackend;
use crate::telemetry::{SERVICE_NAME, SERVICE_VERSION};
use anyhow::{Context, Result};
use async_nats::jetstream::stream::{
    Config as StreamConfig, DiscardPolicy, RetentionPolicy, StorageType,
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

    // Check if stream already exists to avoid unnecessary creation attempts
    match backend.jetstream.get_stream(&stream_name).await {
        Ok(_) => {
            debug!(stream_name = %stream_name, "Stream already exists");
            return Ok(stream_name);
        }
        Err(_) => {
            info!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_domain = "backend",
                event_name = "backend.jetstream.stream.create.started",
                stream_name = %stream_name,
                subject_pattern = %subject_pattern,
                "Creating new stream for base topic"
            );
        }
    }

    // Create stream configuration for this specific base
    let stream_config = build_stream_config_for_base(backend, &stream_name, &subject_pattern)?;

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
                    error = %e,
                    "Failed to create stream"
                );
                Err(e.into())
            }
        }
    }
}

fn build_stream_config_for_base(
    backend: &JetStreamBackend,
    stream_name: &str,
    subject_pattern: &str,
) -> Result<StreamConfig> {
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

    // Create base stream configuration with per-subject message limiting
    let mut config = StreamConfig {
        name: stream_name.to_string(),
        subjects: vec![subject_pattern.to_string()], // Only match this base's topics
        storage: storage_type,
        retention,
        discard,
        max_messages_per_subject: 1, // Keep only the latest message per subject
        ..Default::default()
    };

    // Apply optional limits from configuration
    if let Some(max_messages) = backend.config.max_messages {
        config.max_messages = max_messages;
    }
    if let Some(max_bytes) = backend.config.max_bytes {
        config.max_bytes = max_bytes;
    }
    if let Some(retention_days) = backend.config.retention_days {
        config.max_age = std::time::Duration::from_secs(retention_days as u64 * 24 * 3600);
    }
    if let Some(replicas) = backend.config.replicas {
        config.num_replicas = replicas;
    }

    debug!(
        stream_name = %stream_name,
        subject_pattern = %subject_pattern,
        storage = ?config.storage,
        retention = ?config.retention,
        max_messages_per_subject = config.max_messages_per_subject,
        "Built stream configuration with per-subject limit"
    );

    Ok(config)
}
