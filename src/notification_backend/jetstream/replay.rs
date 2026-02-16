//! JetStream-specific implementation of replay functionality using pull consumers

use anyhow::{Context, Result};
use tokio_stream::StreamExt;
use tracing::{debug, info, warn};

use crate::configuration::Settings;
use crate::notification::topic_parser::derive_stream_name_from_topic;
use crate::notification::wildcard_matcher::{analyze_watch_pattern, matches_watch_pattern};
use crate::notification_backend::jetstream::{
    backend::JetStreamBackend, subscriber_utils::transform_jetstream_message,
};
use crate::notification_backend::replay::{BatchParams, StartAt};
use crate::types::{BatchResult, ReplayLimitInfo};

/// Retrieve a batch of historical messages from JetStream using pull consumer
///
/// This function uses a pull consumer to fetch available messages without hanging,
/// preventing issues when fewer messages exist than requested.
pub async fn get_messages_batch(
    backend: &JetStreamBackend,
    params: BatchParams,
) -> Result<BatchResult> {
    let (backend_pattern, app_filter_pattern) = analyze_watch_pattern(&params.topic)?;

    debug!(
        topic = %params.topic,
        backend_pattern = %backend_pattern,
        start_at = ?params.start_at,
        limit = params.limit,
        "Starting JetStream batch retrieval with deterministic approach"
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
    let consumer =
        create_pull_consumer(backend, &stream_name, &backend_pattern, params.start_at).await?;

    // Fetch messages using batch method
    let mut messages = consumer
        .fetch()
        .max_messages(params.limit)
        .messages()
        .await
        .context("Failed to fetch messages")?;

    // Process the fetched messages
    let mut filtered_messages = Vec::new();
    let mut last_processed_sequence = None;

    // Process messages from the fetch result
    while let Some(msg_result) = messages.next().await {
        match msg_result {
            Ok(msg) => {
                // Track the sequence number
                if let Ok(info) = msg.info() {
                    last_processed_sequence = Some(info.stream_sequence);
                }

                match transform_jetstream_message(&msg) {
                    Ok(notification) => {
                        if matches_watch_pattern(&notification.topic, &app_filter_pattern) {
                            filtered_messages.push(notification);
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, subject = %msg.subject, "Failed to transform message");
                    }
                }
            }
            Err(e) => {
                debug!(error = %e, "Message error during batch retrieval");
            }
        }

        // Break if we have enough messages
        if filtered_messages.len() >= params.limit {
            break;
        }
    }

    // Determine if more messages are available using deterministic logic
    let has_more = if let Some(last_seq) = last_processed_sequence {
        // If we processed fewer messages than requested AND we haven't reached the stream's last sequence,
        // there might be more messages (could be filtered out by pattern matching)
        if filtered_messages.len() < params.limit {
            last_seq < stream_info.state.last_sequence
        } else {
            // We got a full batch, assume more might be available
            true
        }
    } else {
        // No messages were processed, no more available
        false
    };

    debug!(
        retrieved_count = filtered_messages.len(),
        requested_limit = params.limit,
        last_processed_sequence = ?last_processed_sequence,
        stream_last_sequence = stream_info.state.last_sequence,
        has_more = has_more,
        "Batch processing completed"
    );

    // Apply replay limiting with user notification
    let watch_config = Settings::get_global_watch_settings();
    let effective_limit = params.limit.min(watch_config.max_historical_notifications);

    let was_replay_limited = filtered_messages.len() > effective_limit;

    if was_replay_limited {
        warn!(
            retrieved_messages = filtered_messages.len(),
            max_allowed = effective_limit,
            topic = %params.topic,
            "Replay message count limit is reached"
        );
    }

    filtered_messages.truncate(effective_limit);

    // Create batch result with replay limiting information
    let mut batch_result = BatchResult::new(filtered_messages, params.limit);
    batch_result.has_more = has_more && !was_replay_limited; // No more if replay limited
    batch_result.next_sequence = last_processed_sequence.map(|seq| seq + 1);

    // Add replay limiting metadata if applicable
    if was_replay_limited {
        batch_result.replay_limit = Some(ReplayLimitInfo {
            max_allowed: watch_config.max_historical_notifications,
        });
    }

    info!(
        topic = %params.topic,
        stream_name = %stream_name,
        retrieved_count = batch_result.batch_size,
        has_more = batch_result.has_more,
        last_sequence = ?batch_result.last_sequence,
        "JetStream batch retrieval completed using deterministic approach"
    );

    Ok(batch_result)
}

/// Create an ephemeral pull consumer for batch retrieval
async fn create_pull_consumer(
    backend: &JetStreamBackend,
    stream_name: &str,
    backend_pattern: &str,
    start_at: StartAt,
) -> Result<async_nats::jetstream::consumer::Consumer<async_nats::jetstream::consumer::pull::Config>>
{
    use async_nats::jetstream::consumer::{AckPolicy, ReplayPolicy};

    let deliver_policy = determine_deliver_policy(start_at)?;

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

fn determine_deliver_policy(
    start_at: StartAt,
) -> Result<async_nats::jetstream::consumer::DeliverPolicy> {
    use async_nats::jetstream::consumer::DeliverPolicy;

    match start_at {
        StartAt::Date(start_date) => {
            let nanos = start_date
                .timestamp_nanos_opt()
                .context("from_date is outside supported timestamp range")?;
            let start_time = time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(nanos))
                .context("from_date could not be converted to JetStream start time")?;
            debug!(
                start_time = ?start_time,
                "Using ByStartTime delivery policy"
            );
            Ok(DeliverPolicy::ByStartTime { start_time })
        }
        StartAt::Sequence(seq) => match seq {
            0 => {
                debug!("Using DeliverPolicy::All for no replay start parameter");
                Ok(DeliverPolicy::All)
            }
            _ => {
                debug!(
                    start_sequence = seq,
                    "Using ByStartSequence delivery policy"
                );
                Ok(DeliverPolicy::ByStartSequence {
                    start_sequence: seq,
                })
            }
        },
        StartAt::LiveOnly => {
            debug!("Using DeliverPolicy::All for no replay start parameter");
            Ok(DeliverPolicy::All)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::determine_deliver_policy;
    use crate::notification_backend::replay::StartAt;
    use chrono::{DateTime, Utc};

    #[test]
    fn policy_prefers_sequence_when_both_sequence_and_date_present() {
        // Internal replay pagination advances with sequence once batches begin.
        let deliver_policy = determine_deliver_policy(StartAt::Sequence(42)).unwrap();

        assert!(matches!(
            deliver_policy,
            async_nats::jetstream::consumer::DeliverPolicy::ByStartSequence { start_sequence: 42 }
        ));
    }

    #[test]
    fn policy_uses_start_time_when_only_date_is_present() {
        let boundary = DateTime::parse_from_rfc3339("2025-06-09T13:15:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let deliver_policy = determine_deliver_policy(StartAt::Date(boundary)).unwrap();

        assert!(matches!(
            deliver_policy,
            async_nats::jetstream::consumer::DeliverPolicy::ByStartTime { .. }
        ));
    }

    #[test]
    fn policy_uses_all_when_no_replay_parameters_are_present() {
        let deliver_policy = determine_deliver_policy(StartAt::LiveOnly).unwrap();

        assert!(matches!(
            deliver_policy,
            async_nats::jetstream::consumer::DeliverPolicy::All
        ));
    }
}
