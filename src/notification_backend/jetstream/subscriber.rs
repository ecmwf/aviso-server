use crate::notification::topic_parser::derive_stream_name_from_topic;
use crate::notification::wildcard_matcher::analyze_watch_pattern;
use crate::notification_backend::NotificationMessage;
use crate::notification_backend::jetstream::backend::JetStreamBackend;
use crate::notification_backend::jetstream::subscriber_utils::{
    ConsumerConfig, apply_message_filter, create_jetstream_consumer, transform_jetstream_message,
};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use futures::StreamExt;
use futures_util::Stream;
use std::time::Duration;
use tracing::{debug, error, info, warn};

/// Subscribes to a topic pattern using JetStream with hybrid wildcard filtering
///
/// This implementation uses a two-tier filtering approach:
/// JetStream backend filtering: Coarse filtering using JetStream's native subject patterns
/// Application-level filtering: Fine-grained wildcard matching on received messages
pub async fn subscribe_to_topic(
    backend: &JetStreamBackend,
    topic: &str,
) -> Result<Box<dyn Stream<Item = NotificationMessage> + Send + Unpin>> {
    info!(topic = %topic, "Starting subscription to topic with hybrid wildcard filtering");

    // Get resilience settings from backend config (your actual structure)
    let config = &backend.config;

    // Determine retry parameters based on configuration
    let max_attempts = if config.enable_auto_reconnect {
        config.max_reconnect_attempts
    } else {
        1 // Single attempt if auto-reconnect disabled
    };

    let base_delay = config.reconnect_delay_ms;

    // Convert topic to owned string to avoid lifetime issues
    let topic_owned = topic.to_string();

    // Try to create subscription with configured retry behavior
    for attempt in 1..=max_attempts {
        match create_subscription_internal(backend, &topic_owned).await {
            Ok(stream) => {
                if attempt > 1 {
                    info!(
                        topic = %topic_owned,
                        attempt = attempt,
                        "Successfully created subscription after retry"
                    );
                } else {
                    info!(
                        topic = %topic_owned,
                        "Successfully created subscription"
                    );
                }
                return Ok(stream);
            }
            Err(e) => {
                if attempt == max_attempts {
                    error!(
                        error = %e,
                        topic = %topic_owned,
                        attempts = max_attempts,
                        auto_reconnect = config.enable_auto_reconnect,
                        "Failed to create subscription after all attempts"
                    );
                    return Err(e);
                }

                warn!(
                    error = %e,
                    topic = %topic_owned,
                    attempt = attempt,
                    max_attempts = max_attempts,
                    "Subscription failed, retrying..."
                );

                // Simple exponential backoff
                let delay = base_delay * attempt as u64;
                tokio::time::sleep(Duration::from_millis(delay)).await;
            }
        }
    }

    unreachable!()
}

/// Internal subscription implementation
async fn create_subscription_internal(
    backend: &JetStreamBackend,
    topic: &str,
) -> Result<Box<dyn Stream<Item = NotificationMessage> + Send + Unpin>> {
    let (backend_pattern, app_filter_pattern) = analyze_watch_pattern(topic);

    debug!(
        topic = %topic,
        backend_pattern = %backend_pattern,
        app_filter_pattern = ?app_filter_pattern,
        "Analyzed topic pattern for hybrid filtering strategy"
    );

    backend
        .ensure_stream_for_topic(&backend_pattern)
        .await
        .context("Failed to ensure stream exists for topic subscription")?;

    let stream_name = derive_stream_name_from_topic(&backend_pattern)
        .context("Failed to derive stream name from backend pattern")?;

    let consumer_config = ConsumerConfig::for_subscription(&stream_name, &backend_pattern);

    let consumer =
        create_jetstream_consumer(backend, &consumer_config, &stream_name, &backend_pattern)
            .await?;

    let topic_for_closure = topic.to_string();
    let message_stream = consumer
        .messages()
        .await
        .context("Failed to get message stream from consumer")?
        .filter_map(move |msg_result| {
            let app_filter_pattern = app_filter_pattern.clone();
            let topic_for_closure = topic_for_closure.clone();

            async move {
                match msg_result {
                    Ok(jetstream_msg) => match transform_jetstream_message(&jetstream_msg) {
                        Ok(notification_message) => {
                            apply_message_filter(notification_message, &app_filter_pattern)
                        }
                        Err(e) => {
                            warn!(
                                error = %e,
                                topic = %topic_for_closure,
                                subject = %jetstream_msg.subject,
                                "Failed to transform JetStream message, skipping"
                            );
                            None
                        }
                    },
                    Err(e) => {
                        debug!(
                            error = %e,
                            topic = %topic_for_closure,
                            "JetStream message error"
                        );
                        None
                    }
                }
            }
        });

    info!(
        topic = %topic,
        backend_pattern = %backend_pattern,
        stream_name = %stream_name,
        "Successfully created subscription with hybrid filtering"
    );

    Ok(Box::new(Box::pin(message_stream)))
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
