//! Historical message replay streaming functionality

use actix_web::{HttpResponse, web};
use anyhow::Result;
use chrono::Utc;
use futures_util::StreamExt as FuturesStreamExt;
use futures_util::stream::unfold;
use std::sync::Arc;
use tokio::time::Duration;
use tokio_stream::StreamExt as TokioStreamExt;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use super::helpers::{
    apply_stream_lifecycle, create_heartbeat_stream, create_sse_response, frame_to_sse_bytes,
};
use super::types::{ControlEvent, DeliveryKind, StreamFrame};
use crate::configuration::Settings;
use crate::notification::IdentifierConstraint;
use crate::notification::decode_subject_for_display;
use crate::notification::wildcard_matcher::matches_notification_filters;
use crate::notification_backend::{
    NotificationBackend, NotificationMessage,
    replay::{BatchParams, StartAt},
};
use crate::telemetry::{SERVICE_NAME, SERVICE_VERSION};

/// Create a stream that replays historical messages using tokio_stream
///
/// - Fetches batch_size and batch_delay_ms from global configuration
/// - Performs paginated fetch of notifications from the backend
/// - Applies request-level filtering (including optional spatial filtering)
pub fn create_historical_replay_stream(
    topic: String,
    backend: Arc<dyn NotificationBackend>,
    start_at: StartAt,
    request_params: Arc<std::collections::HashMap<String, String>>,
    request_constraints: Arc<std::collections::HashMap<String, IdentifierConstraint>>,
) -> impl tokio_stream::Stream<Item = StreamFrame> {
    // Fetch configuration values from global settings
    let watch_config = Settings::get_global_watch_settings();

    // Build the initial pagination params based on either sequence or date
    let initial_params =
        BatchParams::new(topic.clone(), watch_config.replay_batch_size).with_start_at(start_at);

    // All state is in the tuple: (backend, params, has_more, delay_ms, request_params)
    unfold(
        (
            backend,
            initial_params,
            true,
            watch_config.replay_batch_delay_ms,
            request_params,
            request_constraints,
        ),
        move |(
            backend,
            mut params,
            mut has_more,
            delay_ms,
            request_params,
            request_constraints,
        )| async move {
            if !has_more {
                // End of stream: terminate unfold
                return None;
            }

            // Fetch next batch of messages from backend
            match backend.get_messages_batch(params.clone()).await {
                Ok(batch_result) => {
                    debug!(
                        topic = %decode_subject_for_display(&params.topic),
                        batch_size = batch_result.batch_size,
                        has_more = batch_result.has_more,
                        last_sequence = ?batch_result.last_sequence,
                        "Retrieved historical message batch"
                    );

                    // Update pagination state for next batch
                    has_more = batch_result.has_more;
                    if let Some(next_seq) = batch_result.next_sequence {
                        params = params.with_sequence(next_seq);
                    }

                    // Filter and convert batch to SSE events
                    let mut frames = Vec::new();

                    for message in batch_result.messages {
                        // Filtering: Only send if message matches request fields (including spatial)
                        if !matches_notification_filters(
                            &message.topic,
                            &request_params,
                            &request_constraints,
                            message.metadata.as_ref(),
                            &message.payload,
                        ) {
                            continue;
                        }
                        // Passed all filters: convert to SSE event
                        frames.push(StreamFrame::Notification {
                            notification: message,
                            kind: DeliveryKind::Replay,
                        });
                    }

                    // If there was a replay limit, inform client with control event
                    if let Some(replay_limit_info) = &batch_result.replay_limit {
                        frames.push(StreamFrame::Control(ControlEvent::ReplayLimitReached {
                            topic: params.topic.clone(),
                            max_allowed: replay_limit_info.max_allowed,
                            timestamp: Utc::now(),
                        }));
                    }

                    // Optional batch delay for rate limiting
                    if delay_ms > 0 && has_more {
                        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    }

                    // Return current batch frames and updated replay state.
                    Some((
                        tokio_stream::iter(frames),
                        (
                            backend,
                            params,
                            has_more,
                            delay_ms,
                            request_params,
                            request_constraints,
                        ),
                    ))
                }
                Err(e) => {
                    warn!(
                        service_name = SERVICE_NAME,
                        service_version = SERVICE_VERSION,
                        event_name = "stream.replay.batch.failed",
                        error = %e,
                        topic = %decode_subject_for_display(&params.topic),
                        "Failed to retrieve historical message batch"
                    );
                    // On error: emit one error frame and stop replay iteration.
                    let error_frames = vec![StreamFrame::Error {
                        topic: params.topic.clone(),
                        message: e.to_string(),
                    }];
                    Some((
                        tokio_stream::iter(error_frames),
                        (
                            backend,
                            params,
                            false,
                            delay_ms,
                            request_params,
                            request_constraints,
                        ),
                    ))
                }
            }
        },
    )
    .flatten() // Flatten nested batch streams into one continuous stream
}

/// Create a combined stream that transitions from historical to live messages
pub async fn create_historical_then_live_stream(
    topic: String,
    backend: Arc<dyn NotificationBackend>,
    start_at: StartAt,
    shutdown: web::Data<CancellationToken>,
    request_params: Arc<std::collections::HashMap<String, String>>,
    request_constraints: Arc<std::collections::HashMap<String, IdentifierConstraint>>,
    sse_guard: Option<crate::metrics::SseConnectionGuard>,
) -> Result<HttpResponse> {
    let watch_config = Settings::get_global_watch_settings();
    let app_settings = Settings::get_global_application_settings();

    // Create historical replay stream
    let historical_stream = create_historical_replay_stream(
        topic.clone(),
        backend.clone(),
        start_at,
        request_params.clone(),
        request_constraints.clone(),
    );

    let (from_sequence, from_date) = start_at.as_replay_cursor();

    // Create control events for replay lifecycle.
    let start_event = StreamFrame::Control(ControlEvent::ReplayStarted {
        topic: topic.clone(),
        from_sequence,
        from_date,
        batch_size: watch_config.replay_batch_size,
        timestamp: chrono::Utc::now(),
    });
    let completion_event = StreamFrame::Control(ControlEvent::ReplayCompleted {
        topic: topic.clone(),
        timestamp: chrono::Utc::now(),
    });

    // Create live subscription stream with request filtering.
    let notification_stream = backend.subscribe_to_topic(&topic).await?;
    let request_params_clone = request_params.clone();
    let request_constraints_clone = request_constraints.clone();
    let filtered_stream = futures_util::StreamExt::filter_map(
        notification_stream,
        move |message: NotificationMessage| {
            super::live::filter_notification_message(
                message,
                request_params_clone.clone(),
                request_constraints_clone.clone(),
            )
        },
    );

    let live_notification_sse_stream = super::live::create_live_notification_stream(
        filtered_stream,
        watch_config.concurrent_notification_processing,
    );

    // Create heartbeat stream
    let heartbeat_stream =
        create_heartbeat_stream(topic.clone(), watch_config.sse_heartbeat_interval_sec);

    // Sequence: replay_started -> replay frames -> replay_completed -> live frames.
    let combined_notification_stream = FuturesStreamExt::chain(
        FuturesStreamExt::chain(tokio_stream::once(start_event), historical_stream),
        FuturesStreamExt::chain(
            tokio_stream::once(completion_event),
            live_notification_sse_stream,
        ),
    );

    // Merge with heartbeat stream
    let merged_stream = TokioStreamExt::merge(combined_notification_stream, heartbeat_stream);

    // Apply lifecycle and convert typed frames to SSE bytes.
    let stream_with_lifecycle = apply_stream_lifecycle(
        merged_stream,
        topic.clone(),
        shutdown.get_ref().clone(),
        Some(Duration::from_secs(
            watch_config.connection_max_duration_sec,
        )),
    );
    let base_url = app_settings.base_url.clone();
    let byte_stream = FuturesStreamExt::map(stream_with_lifecycle, move |frame| {
        frame_to_sse_bytes(frame, &base_url)
    });

    tracing::info!(
        service_name = SERVICE_NAME,
        service_version = SERVICE_VERSION,
        event_name = "stream.watch.replay_live.created",
        topic = %decode_subject_for_display(&topic),
        from_sequence = ?from_sequence,
        from_date = ?from_date,
        batch_size = watch_config.replay_batch_size,
        "Created combined historical-then-live SSE stream"
    );

    Ok(create_sse_response(byte_stream, sse_guard))
}

/// Create a replay-only stream (historical messages then close)
///
/// This stream ends after replay completion; it does not transition to live notifications.
pub async fn create_replay_only_stream(
    topic: String,
    backend: Arc<dyn NotificationBackend>,
    start_at: StartAt,
    shutdown: web::Data<CancellationToken>,
    request_params: Arc<std::collections::HashMap<String, String>>,
    request_constraints: Arc<std::collections::HashMap<String, IdentifierConstraint>>,
    sse_guard: Option<crate::metrics::SseConnectionGuard>,
) -> Result<HttpResponse> {
    let watch_config = Settings::get_global_watch_settings();

    // Create historical replay stream
    let historical_stream = create_historical_replay_stream(
        topic.clone(),
        backend.clone(),
        start_at,
        request_params.clone(),
        request_constraints.clone(),
    );

    let (from_sequence, from_date) = start_at.as_replay_cursor();

    // Create control events for replay lifecycle.
    let start_event = StreamFrame::Control(ControlEvent::ReplayStarted {
        topic: topic.clone(),
        from_sequence,
        from_date,
        batch_size: watch_config.replay_batch_size,
        timestamp: Utc::now(),
    });
    let completion_event = StreamFrame::Control(ControlEvent::ReplayCompleted {
        topic: topic.clone(),
        timestamp: chrono::Utc::now(),
    });

    // Chain: start -> historical -> completion
    let replay_stream = FuturesStreamExt::chain(
        FuturesStreamExt::chain(tokio_stream::once(start_event), historical_stream),
        tokio_stream::once(completion_event),
    );

    // Replay endpoint is finite; close reason is end_of_stream unless interrupted.
    let stream_with_lifecycle = apply_stream_lifecycle(
        replay_stream,
        topic.clone(),
        shutdown.get_ref().clone(),
        None,
    );
    let app_settings = Settings::get_global_application_settings();
    let base_url = app_settings.base_url.clone();
    let byte_stream = FuturesStreamExt::map(stream_with_lifecycle, move |frame| {
        frame_to_sse_bytes(frame, &base_url)
    });

    tracing::info!(
        service_name = SERVICE_NAME,
        service_version = SERVICE_VERSION,
        event_name = "stream.replay.created",
        topic = %decode_subject_for_display(&topic),
        from_sequence = ?from_sequence,
        from_date = ?from_date,
        batch_size = watch_config.replay_batch_size,
        "Created replay-only SSE stream"
    );

    // Use existing helper for response creation
    Ok(create_sse_response(byte_stream, sse_guard))
}
