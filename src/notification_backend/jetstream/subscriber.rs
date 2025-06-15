use crate::notification::topic_parser::derive_event_type_from_topic;
use crate::notification::wildcard_matcher::{analyze_watch_pattern, matches_watch_pattern};
use crate::notification_backend::NotificationMessage;
use crate::notification_backend::jetstream::backend::JetStreamBackend;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use futures::StreamExt;
use futures_util::Stream;
use tracing::{debug, error, info};

/// Subscribe to a topic with hybrid wildcard filtering.
///
/// This method creates a JetStream consumer with a pull subscription
/// and applies application-level wildcard filtering on received messages.
///
/// The subscription uses a unique consumer name based on the stream and timestamp.
///
/// # Arguments
/// * `backend` - Reference to the JetStreamBackend instance
/// * `topic` - Topic pattern to subscribe to
///
/// # Returns
/// * `Result` with a boxed stream of NotificationMessage items
pub async fn subscribe_to_topic(
    backend: &JetStreamBackend,
    topic: &str,
) -> Result<Box<dyn Stream<Item = NotificationMessage> + Unpin + Send>> {
    // Analyze the watch pattern for hybrid filtering
    let (backend_subscription_pattern, app_filter_pattern) = analyze_watch_pattern(topic);

    info!(
        watch_topic = %topic,
        backend_pattern = %backend_subscription_pattern,
        app_filter_parts = app_filter_pattern.len(),
        "Setting up hybrid wildcard subscription"
    );

    // Extract base from backend pattern for stream name
    let base = derive_event_type_from_topic(&backend_subscription_pattern)
        .context("Failed to extract base from backend subscription pattern")?;

    // Create stream name by uppercasing the base
    let stream_name = base.to_uppercase();

    // Ensure the stream exists before subscribing
    backend
        .ensure_stream_for_topic(&backend_subscription_pattern)
        .await
        .context("Failed to ensure stream exists for subscription")?;

    // Get JetStream context
    let jetstream = async_nats::jetstream::new(backend.client.clone());

    // Create consumer configuration using the backend subscription pattern
    let consumer_config = async_nats::jetstream::consumer::pull::Config {
        name: Some(format!(
            "watch_consumer_{}_{}",
            stream_name,
            Utc::now().timestamp_millis()
        )),
        durable_name: None, // Ephemeral consumer for watch connections
        description: Some(format!(
            "Watch consumer for pattern: {}",
            backend_subscription_pattern
        )),
        // Use the backend subscription pattern for JetStream filtering
        filter_subject: backend_subscription_pattern.clone(),
        deliver_policy: async_nats::jetstream::consumer::DeliverPolicy::New, // Only new messages
        ack_policy: async_nats::jetstream::consumer::AckPolicy::None, // No ack needed for watch
        replay_policy: async_nats::jetstream::consumer::ReplayPolicy::Instant,
        max_deliver: 1,
        ..Default::default()
    };

    // Create the pull consumer
    let consumer = jetstream
        .create_consumer_on_stream(consumer_config, &stream_name)
        .await
        .context("Failed to create JetStream consumer for topic subscription")?;

    info!(
        backend_pattern = %backend_subscription_pattern, // Now this works
        stream_name = %stream_name,
        consumer_name = consumer.cached_info().name,
        "Created JetStream consumer with backend pattern filtering"
    );

    // Get the JetStream message stream
    let message_stream = consumer
        .messages()
        .await
        .context("Failed to get message stream from JetStream consumer")?;

    // Transform JetStream messages with application-level filtering
    let notification_stream = StreamExt::filter_map(message_stream, move |msg_result| {
        let app_filter = app_filter_pattern.clone();
        async move {
            match msg_result {
                Ok(msg) => {
                    // Extract message metadata
                    let sequence = msg.info().unwrap().stream_sequence;
                    let jetstream_timestamp = msg.info().unwrap().published;
                    let subject = msg.subject.to_string();

                    // Convert OffsetDateTime to DateTime<Utc>
                    let timestamp = DateTime::<Utc>::from_timestamp(
                        jetstream_timestamp.unix_timestamp(),
                        jetstream_timestamp.nanosecond(),
                    )
                    .unwrap_or_else(Utc::now);

                    // Apply application-level wildcard filtering
                    if matches_watch_pattern(&subject, &app_filter) {
                        // Convert payload bytes to string
                        let payload = String::from_utf8_lossy(&msg.payload).to_string();

                        // Create NotificationMessage
                        let notification_msg = NotificationMessage {
                            sequence,
                            topic: subject.clone(),
                            payload,
                            timestamp: Some(timestamp),
                        };

                        debug!(
                            topic = %subject,
                            sequence = sequence,
                            timestamp = ?timestamp,
                            "Message passed wildcard filter, delivering to client"
                        );

                        Some(notification_msg)
                    } else {
                        debug!(
                            topic = %subject,
                            sequence = sequence,
                            "Message filtered out by application-level wildcard matching"
                        );
                        None
                    }
                }
                Err(e) => {
                    error!(
                        error = %e,
                        "Error receiving message from JetStream subscription"
                    );
                    // Filter out errors, continue with other messages
                    None
                }
            }
        }
    })
    .boxed();

    debug!(
        watch_topic = %topic,
        backend_pattern = %backend_subscription_pattern,
        stream_name = %stream_name,
        "JetStream subscription stream created with hybrid wildcard filtering"
    );

    Ok(Box::new(notification_stream))
}

/// TODO: Implement JetStream message batch retrieval
#[allow(unused_variables)]
pub async fn get_messages_batch(
    backend: &JetStreamBackend,
    topic: &str,
    from_sequence: Option<u64>,
    from_date: Option<DateTime<Utc>>,
    limit: usize,
    offset: usize,
) -> Result<(Vec<NotificationMessage>, bool)> {
    todo!("JetStream get_messages_batch not yet implemented")
}

/// TODO: Implement JetStream message counting
#[allow(unused_variables)]
pub async fn count_messages(
    backend: &JetStreamBackend,
    topic: &str,
    from_sequence: Option<u64>,
    from_date: Option<DateTime<Utc>>,
) -> Result<usize> {
    todo!("JetStream count_messages not yet implemented")
}
