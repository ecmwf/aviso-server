// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! JetStream-specific implementation of replay functionality using pull consumers

use anyhow::{Context, Result};
use tokio_stream::StreamExt;
use tracing::{debug, info, warn};

use crate::notification::decode_subject_for_display;
use crate::notification::topic_parser::derive_stream_name_from_topic;
use crate::notification::wildcard_matcher::{analyze_watch_pattern, matches_watch_pattern};
use crate::notification_backend::jetstream::{
    backend::JetStreamBackend, subscriber_utils::transform_jetstream_message,
};
use crate::notification_backend::replay::{BatchParams, StartAt};
use crate::telemetry::{SERVICE_NAME, SERVICE_VERSION};
use crate::types::BatchResult;

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
    let end_sequence = params.end_sequence.min(stream_info.state.last_sequence);
    if stream_info.state.messages == 0
        || end_sequence == 0
        || matches!(params.start_at, StartAt::Sequence(start) if start > end_sequence)
    {
        debug!("No messages available in stream");
        return Ok(BatchResult::empty());
    }

    // Create ephemeral pull consumer
    let consumer =
        create_pull_consumer(backend, &stream_name, &backend_pattern, params.start_at).await?;

    // Fetch messages using batch method
    let messages = consumer
        .fetch()
        .max_messages(params.limit)
        .messages()
        .await
        .context("Failed to fetch messages")?;

    let batch_result =
        read_replay_batch(messages, &app_filter_pattern, params.limit, end_sequence).await?;

    info!(
        service_name = SERVICE_NAME,
        service_version = SERVICE_VERSION,
        event_name = "backend.jetstream.replay.batch.succeeded",
        topic = %decode_subject_for_display(&params.topic),
        stream_name = %stream_name,
        retrieved_count = batch_result.batch_size,
        has_more = batch_result.has_more,
        last_sequence = ?batch_result.last_sequence,
        "JetStream batch retrieval completed using deterministic approach"
    );

    Ok(batch_result)
}

async fn read_replay_batch(
    mut messages: impl tokio_stream::Stream<
        Item = std::result::Result<async_nats::jetstream::Message, async_nats::Error>,
    > + Unpin,
    app_filter_pattern: &[String],
    limit: usize,
    end_sequence: u64,
) -> Result<BatchResult> {
    // Process the fetched messages
    let mut filtered_messages = Vec::new();
    let mut last_processed_sequence = None;
    let mut reached_end = false;

    // Process messages from the fetch result
    while let Some(msg_result) = messages.next().await {
        let msg = msg_result
            .map_err(anyhow::Error::from_boxed)
            .context("Failed to read JetStream replay batch")?;
        let sequence = msg
            .info()
            .map_err(anyhow::Error::from_boxed)
            .context("Failed to read replay sequence")?
            .stream_sequence;
        if sequence > end_sequence {
            reached_end = true;
            break;
        }
        last_processed_sequence = Some(sequence);

        match transform_jetstream_message(&msg) {
            Ok(notification) => {
                if matches_watch_pattern(&notification.topic, app_filter_pattern) {
                    filtered_messages.push(notification);
                }
            }
            Err(e) => {
                warn!(
                    service_name = SERVICE_NAME,
                    service_version = SERVICE_VERSION,
                    event_name = "backend.jetstream.replay.message_transform.failed",
                    error = %e,
                    subject = %msg.subject,
                    "Failed to transform message"
                );
            }
        }

        // The bound is inclusive, even when this message was filtered out.
        if sequence == end_sequence {
            reached_end = true;
            break;
        }
        if filtered_messages.len() >= limit {
            break;
        }
    }

    // Determine if more messages are available using deterministic logic
    let has_more = !reached_end && last_processed_sequence.is_some_and(|seq| seq < end_sequence);

    debug!(
        retrieved_count = filtered_messages.len(),
        requested_limit = limit,
        last_processed_sequence = ?last_processed_sequence,
        end_sequence,
        has_more = has_more,
        "Batch processing completed"
    );

    // Request-wide replay quotas are applied after SSE request filtering.
    let mut batch_result = BatchResult::new(filtered_messages, limit);
    batch_result.has_more = has_more;
    batch_result.next_sequence = last_processed_sequence.map(|seq| seq + 1);

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

    // Create consumer configuration for batch retrieval.
    // UUID suffix prevents collision on concurrent same-event_type replays
    // (same root cause as the watch consumer naming, see subscriber_utils.rs).
    let consumer_config = async_nats::jetstream::consumer::pull::Config {
        name: Some(format!(
            "replay_consumer_{}_{}",
            chrono::Utc::now().timestamp_millis(),
            uuid::Uuid::new_v4().simple()
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
        service_name = SERVICE_NAME,
        service_version = SERVICE_VERSION,
        event_name = "backend.jetstream.replay.consumer.created",
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

    #[tokio::test]
    async fn fetch_stream_error_is_not_empty_exhaustion() {
        let error: async_nats::Error = Box::new(std::io::Error::other("consumer revoked"));
        let messages = tokio_stream::StreamExt::chain(
            tokio_stream::once(Err(error)),
            futures_util::stream::poll_fn(|_| panic!("must stop reading after fetch error")),
        );
        let error = super::read_replay_batch(messages, &["test".into(), "*".into()], 1, 2)
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("consumer revoked"));
    }

    #[tokio::test]
    async fn clean_fetch_stream_exhaustion_is_empty() {
        let batch = super::read_replay_batch(tokio_stream::empty(), &[], 1, 2)
            .await
            .unwrap();
        assert!(batch.messages.is_empty());
        assert!(!batch.has_more);
    }

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
