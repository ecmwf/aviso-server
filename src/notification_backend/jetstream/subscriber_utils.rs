//! Utility functions for JetStream subscriber operations
//!
//! This module contains shared utilities for subscription and message processing
//! that can be reused across different JetStream operations like subscribe_to_topic
//! and future get_messages_batch functionality.

use anyhow::{Context, Result};
use async_nats::jetstream::consumer::Consumer;
use chrono::{DateTime, Utc};
use tracing::{debug, info};

use crate::notification_backend::{JetStreamBackend, NotificationMessage};
use crate::telemetry::{SERVICE_NAME, SERVICE_VERSION};

/// Configuration for different types of JetStream consumers
#[derive(Debug, Clone)]
pub struct ConsumerConfig {
    /// Consumer name prefix
    pub name_prefix: String,
    /// Whether this is a durable consumer
    pub durable: bool,
    /// Delivery policy for message retrieval
    pub deliver_policy: async_nats::jetstream::consumer::DeliverPolicy,
    /// Acknowledgment policy
    pub ack_policy: async_nats::jetstream::consumer::AckPolicy,
    /// Replay policy for historical messages
    pub replay_policy: async_nats::jetstream::consumer::ReplayPolicy,
    /// Maximum delivery attempts
    pub max_deliver: i64,
    /// Optional description
    pub description: Option<String>,
}

impl ConsumerConfig {
    /// Create configuration for real-time subscription consumers
    /// These are ephemeral consumers that only receive new messages
    ///
    /// # Arguments
    /// * `stream_name` - Name of the stream
    /// * `backend_pattern` - JetStream subject filter pattern
    ///
    /// # Returns
    /// * `Self` - Configuration optimized for real-time subscriptions
    pub fn for_subscription(stream_name: &str, backend_pattern: &str) -> Self {
        Self {
            name_prefix: format!("watch_consumer_{}", stream_name),
            durable: false,
            deliver_policy: async_nats::jetstream::consumer::DeliverPolicy::New,
            ack_policy: async_nats::jetstream::consumer::AckPolicy::None,
            replay_policy: async_nats::jetstream::consumer::ReplayPolicy::Instant,
            max_deliver: 1,
            description: Some(format!("Watch consumer for pattern: {}", backend_pattern)),
        }
    }
}

/// Creates a JetStream consumer configuration from our internal config
/// Handles the conversion between our abstraction and JetStream's native config
///
/// # Arguments
/// * `config` - Our internal consumer configuration
/// * `backend_pattern` - JetStream subject filter pattern
///
/// # Returns
/// * `async_nats::jetstream::consumer::pull::Config` - Native JetStream config
pub fn build_jetstream_consumer_config(
    config: &ConsumerConfig,
    backend_pattern: &str,
) -> async_nats::jetstream::consumer::pull::Config {
    let consumer_name = Some(format!(
        "{}_{}",
        config.name_prefix,
        Utc::now().timestamp_millis()
    ));

    let durable_name = if config.durable {
        Some(format!("{}_durable", config.name_prefix))
    } else {
        None
    };

    async_nats::jetstream::consumer::pull::Config {
        name: consumer_name,
        durable_name,
        description: config.description.clone(),
        filter_subject: backend_pattern.to_string(),
        deliver_policy: config.deliver_policy,
        ack_policy: config.ack_policy,
        replay_policy: config.replay_policy,
        max_deliver: config.max_deliver,
        ..Default::default()
    }
}

/// Creates a JetStream consumer with the specified configuration
/// Handles the consumer creation and provides detailed logging
///
/// # Arguments
/// * `backend` - Reference to the JetStreamBackend
/// * `config` - Consumer configuration
/// * `stream_name` - Name of the target stream
/// * `backend_pattern` - JetStream subject filter pattern
///
/// # Returns
/// * `Result<Consumer>` - Created consumer or error
pub async fn create_jetstream_consumer(
    backend: &JetStreamBackend,
    config: &ConsumerConfig,
    stream_name: &str,
    backend_pattern: &str,
) -> Result<Consumer<async_nats::jetstream::consumer::pull::Config>> {
    // Build JetStream consumer configuration
    let jetstream_config = build_jetstream_consumer_config(config, backend_pattern);

    // Create the pull consumer
    let consumer = backend
        .jetstream
        .create_consumer_on_stream(jetstream_config, stream_name)
        .await
        .context("Failed to create JetStream consumer")?;

    info!(
        service_name = SERVICE_NAME,
        service_version = SERVICE_VERSION,
        event_domain = "backend",
        event_name = "backend.jetstream.consumer.created",
        backend_pattern = %backend_pattern,
        stream_name = %stream_name,
        consumer_name = consumer.cached_info().name,
        consumer_type = if config.durable { "durable" } else { "ephemeral" },
        "Created JetStream consumer with backend pattern filtering"
    );

    Ok(consumer)
}

/// Transforms a JetStream message into our NotificationMessage format
/// Handles timestamp conversion and payload extraction
///
/// # Arguments
/// * `jetstream_msg` - The raw JetStream message
///
/// # Returns
/// * `Result<NotificationMessage>` - Converted message or error
pub fn transform_jetstream_message(
    jetstream_msg: &async_nats::jetstream::Message,
) -> Result<NotificationMessage> {
    // Extract message metadata - handle the Result properly
    let info = jetstream_msg
        .info()
        .map_err(|e| anyhow::anyhow!("Failed to get message info from JetStream message: {}", e))?;

    let sequence = info.stream_sequence;
    let jetstream_timestamp = info.published;
    let subject = jetstream_msg.subject.to_string();

    // Convert OffsetDateTime to DateTime<Utc>
    let timestamp = DateTime::<Utc>::from_timestamp(
        jetstream_timestamp.unix_timestamp(),
        jetstream_timestamp.nanosecond(),
    )
    .unwrap_or_else(Utc::now);

    // Convert payload bytes to string
    let payload = String::from_utf8_lossy(&jetstream_msg.payload).to_string();

    let metadata = match &jetstream_msg.headers {
        Some(headers) => {
            let mut map = std::collections::HashMap::new();
            for (k, v) in headers.iter() {
                // Each header value is Vec<String>, join with ','
                let value = v.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(",");
                map.insert(k.to_string(), value);
            }
            if map.is_empty() { None } else { Some(map) }
        }
        None => None,
    };

    debug!(
        sequence = sequence,
        topic = %subject,
        headers_present = metadata.is_some(),
        "JetStream message converted, headers extracted"
    );

    // Create NotificationMessage
    Ok(NotificationMessage {
        sequence,
        topic: subject,
        payload,
        timestamp: Some(timestamp),
        metadata,
    })
}

/// Applies application-level wildcard filtering to a message
/// Returns Some(message) if it passes the filter, None if it should be filtered out
///
/// # Arguments
/// * `message` - The notification message to filter
/// * `app_filter_pattern` - Application-level filter parts from analyze_watch_pattern
///
/// # Returns
/// * `Option<NotificationMessage>` - Message if it passes filter, None otherwise
pub fn apply_message_filter(
    message: NotificationMessage,
    app_filter_pattern: &[String],
) -> Option<NotificationMessage> {
    if crate::notification::wildcard_matcher::matches_watch_pattern(
        &message.topic,
        app_filter_pattern,
    ) {
        debug!(
            topic = %message.topic,
            sequence = message.sequence,
            timestamp = ?message.timestamp,
            "Message passed wildcard filter, delivering to client"
        );
        Some(message)
    } else {
        debug!(
            topic = %message.topic,
            sequence = message.sequence,
            "Message filtered out by application-level wildcard matching"
        );
        None
    }
}
