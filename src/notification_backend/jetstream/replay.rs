//! JetStream-specific implementation of replay functionality using pull consumers

use anyhow::{Context, Result};
use tracing::{debug, info, warn};

use crate::notification::topic_parser::derive_stream_name_from_topic;
use crate::notification::wildcard_matcher::analyze_watch_pattern;
use crate::notification_backend::jetstream::{
    backend::JetStreamBackend, subscriber_utils::transform_jetstream_message,
};
use crate::notification_backend::replay::BatchParams;
use crate::types::{BatchResult, RateLimitInfo};
use futures_util::StreamExt;

/// Retrieve a batch of historical messages from JetStream using pull consumer
///
/// This function uses a pull consumer to fetch available messages without hanging,
/// preventing issues when fewer messages exist than requested.
pub async fn get_messages_batch(
    backend: &JetStreamBackend,
    params: BatchParams,
) -> Result<BatchResult> {
    let (backend_pattern, app_filter_pattern) = analyze_watch_pattern(&params.topic);

    debug!(
        topic = %params.topic,
        backend_pattern = %backend_pattern,
        from_sequence = ?params.from_sequence,
        limit = params.limit,
        "Starting JetStream batch retrieval with pull consumer"
    );

    // Ensure stream exists for the topic
    backend
        .ensure_stream_for_topic(&backend_pattern)
        .await
        .context("Failed to ensure stream exists for batch retrieval")?;

    let stream_name = derive_stream_name_from_topic(&backend_pattern)
        .context("Failed to derive stream name from topic")?;

    // Get stream info to check if messages are available
    let mut stream = backend
        .jetstream
        .get_stream(&stream_name)
        .await
        .context("Failed to get stream")?;

    let stream_info = stream.info().await.context("Failed to get stream info")?;

    debug!(
        stream_name = %stream_name,
        total_messages = stream_info.state.messages,
        first_sequence = stream_info.state.first_sequence,
        last_sequence = stream_info.state.last_sequence,
        "Stream info retrieved"
    );

    // Check if stream has any messages
    if stream_info.state.messages == 0 {
        debug!("No messages available in stream");
        return Ok(BatchResult::empty());
    }

    // Create ephemeral pull consumer
    let consumer = create_pull_consumer(
        backend,
        &stream_name,
        &backend_pattern,
        params.from_sequence,
    )
    .await?;

    // Fetch messages using batch method
    let mut message_stream = consumer
        .batch()
        .max_messages(params.limit)
        .messages()
        .await
        .context("Failed to get message stream")?;

    // Process available messages with timeout to prevent hanging
    let mut filtered_messages = Vec::new();
    let mut processed_count = 0;

    while processed_count < params.limit {
        // Use timeout to prevent hanging when no more messages are available
        match tokio::time::timeout(std::time::Duration::from_millis(100), message_stream.next())
            .await
        {
            Ok(Some(message_result)) => {
                match message_result {
                    Ok(msg) => match transform_jetstream_message(&msg) {
                        Ok(notification) => {
                            if crate::notification::wildcard_matcher::matches_watch_pattern(
                                &notification.topic,
                                &app_filter_pattern,
                            ) {
                                filtered_messages.push(notification);
                            }
                        }
                        Err(e) => {
                            warn!(error = %e, subject = %msg.subject, "Failed to transform message");
                        }
                    },
                    Err(e) => {
                        debug!(error = %e, "Message error during batch retrieval");
                    }
                }
                processed_count += 1;
            }
            Ok(None) => {
                debug!("No more messages available in stream");
                break;
            }
            Err(_) => {
                debug!("Timeout waiting for messages - assuming no more available");
                break;
            }
        }
    }

    // Apply rate limiting
    let watch_config = crate::configuration::Settings::get_global_watch_settings();
    let effective_limit = params.limit.min(watch_config.max_historical_notifications);

    let was_rate_limited = filtered_messages.len() > effective_limit;
    let original_count = filtered_messages.len();

    if was_rate_limited {
        warn!(
            requested_messages = original_count,
            max_allowed = effective_limit,
            topic = %params.topic,
            "Rate limiting applied to historical replay"
        );
    }
    filtered_messages.truncate(effective_limit);

    let mut batch_result = BatchResult::new(filtered_messages, params.limit);

    // Add rate limiting metadata if applicable
    if was_rate_limited {
        batch_result.rate_limited = Some(RateLimitInfo {
            original_count,
            max_allowed: watch_config.max_historical_notifications,
        });
    }

    info!(
        topic = %params.topic,
        stream_name = %stream_name,
        retrieved_count = batch_result.batch_size,
        has_more = batch_result.has_more,
        last_sequence = ?batch_result.last_sequence,
        "JetStream batch retrieval completed"
    );

    Ok(batch_result)
}

/// Create an ephemeral pull consumer for batch retrieval
async fn create_pull_consumer(
    backend: &JetStreamBackend,
    stream_name: &str,
    backend_pattern: &str,
    from_sequence: Option<u64>,
) -> Result<async_nats::jetstream::consumer::Consumer<async_nats::jetstream::consumer::pull::Config>>
{
    use async_nats::jetstream::consumer::{AckPolicy, DeliverPolicy, ReplayPolicy};

    // Fix: Handle all possible u64 values in the match
    let deliver_policy = match from_sequence {
        Some(0) | None => {
            debug!("Using DeliverPolicy::All for sequence 0 or no sequence");
            DeliverPolicy::All
        }
        Some(seq) => {
            debug!(
                start_sequence = seq,
                "Using ByStartSequence delivery policy"
            );
            DeliverPolicy::ByStartSequence {
                start_sequence: seq,
            }
        }
    };

    // Create consumer configuration for batch retrieval
    let consumer_config = async_nats::jetstream::consumer::pull::Config {
        name: Some(format!(
            "replay_consumer_{}",
            chrono::Utc::now().timestamp_millis()
        )),
        durable_name: None, // Ephemeral consumer
        description: Some(format!("Replay consumer for pattern: {}", backend_pattern)),
        filter_subject: backend_pattern.to_string(),
        deliver_policy,
        ack_policy: AckPolicy::None,          // Read-only for replay
        replay_policy: ReplayPolicy::Instant, // Fast replay
        max_deliver: 1,
        ..Default::default()
    };

    debug!(
        consumer_config = ?consumer_config,
        "Creating pull consumer with configuration"
    );

    // Create the pull consumer
    let consumer = backend
        .jetstream
        .create_consumer_on_stream(consumer_config, stream_name)
        .await
        .context("Failed to create JetStream consumer for batch retrieval")?;

    info!(
        stream_name = %stream_name,
        backend_pattern = %backend_pattern,
        deliver_policy = ?deliver_policy,
        consumer_name = consumer.cached_info().name,
        "Successfully created ephemeral pull consumer"
    );

    Ok(consumer)
}
