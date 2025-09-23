//! Historical message replay streaming functionality

use actix_web::{HttpResponse, web};
use anyhow::Result;
use chrono::{DateTime, Utc};
use futures_util::StreamExt as FuturesStreamExt;
use futures_util::stream::unfold;
use serde_json::json;
use std::sync::Arc;
use tokio::time::Duration;
use tokio_stream::StreamExt as TokioStreamExt;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use super::helpers::{
    apply_graceful_shutdown, create_heartbeat_stream, create_sse_response,
    notification_to_sse_event,
};
use super::types::{SseEventType, format_sse_event};
use crate::configuration::Settings;
use crate::notification::wildcard_matcher::matches_notification_filters;
use crate::notification_backend::{NotificationBackend, NotificationMessage, replay::BatchParams};

/// Create a stream that replays historical messages using tokio_stream
///
/// - Fetches batch_size, batch_delay_ms, and base_url from global configuration
/// - Performs paginated fetch of notifications from the backend
/// - Applies spatial and parameter filtering using canonicalized request params
pub fn create_historical_replay_stream(
    topic: String,
    backend: Arc<dyn NotificationBackend>,
    from_sequence: Option<u64>,
    from_date: Option<DateTime<Utc>>,
    request_params: Arc<std::collections::HashMap<String, String>>,
) -> impl tokio_stream::Stream<Item = Result<web::Bytes, actix_web::Error>> {
    // Fetch configuration values from global settings
    let watch_config = Settings::get_global_watch_settings();
    let app_settings = Settings::get_global_application_settings();

    // Build the initial pagination params based on either sequence or date
    let initial_params = BatchParams::new(topic.clone(), watch_config.replay_batch_size);
    let initial_params = if let Some(seq) = from_sequence {
        initial_params.with_sequence(seq)
    } else if let Some(date) = from_date {
        initial_params.with_date(date)
    } else {
        initial_params
    };

    // All state is in the tuple: (backend, params, has_more, base_url, delay_ms, request_params)
    unfold(
        (
            backend,
            initial_params,
            true,
            app_settings.base_url.clone(),
            watch_config.replay_batch_delay_ms,
            request_params,
        ),
        move |(backend, mut params, mut has_more, base_url, delay_ms, request_params)| async move {
            if !has_more {
                // End of stream: terminate unfold
                return None;
            }

            // Fetch next batch of messages from backend
            match backend.get_messages_batch(params.clone()).await {
                Ok(batch_result) => {
                    debug!(
                        topic = %params.topic,
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
                    let mut sse_events = Vec::new();

                    for message in batch_result.messages {
                        // Filtering: Only send if message matches request fields (including spatial)
                        if !matches_notification_filters(
                            &request_params,
                            message.metadata.as_ref(),
                            &message.payload,
                        ) {
                            continue;
                        }
                        // Passed all filters: convert to SSE event
                        let sse_event = notification_to_sse_event(
                            &message,
                            &base_url,
                            SseEventType::ReplayNotification,
                        );
                        sse_events.push(sse_event);
                    }

                    // If there was a replay limit, inform client with control event
                    if let Some(replay_limit_info) = &batch_result.replay_limit {
                        let replay_limit_event = format_sse_event(
                            SseEventType::ReplayControl,
                            json!({
                                "type": "notification_replay_limit_reached",
                                "topic": params.topic,
                                "max_allowed": replay_limit_info.max_allowed,
                                "message": format!(
                                    "Historical replay limited to {} messages. \
                                    Additional historical messages may be available but were not retrieved.",
                                    replay_limit_info.max_allowed
                                ),
                                "timestamp": Utc::now().to_rfc3339()
                             }),
                        );
                        sse_events.push(Ok(web::Bytes::from(replay_limit_event)));
                    }

                    // Optional batch delay for rate limiting
                    if delay_ms > 0 && has_more {
                        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    }

                    // Return the stream of SSE events and updated state tuple (including filtering params)
                    Some((
                        tokio_stream::iter(sse_events),
                        (backend, params, has_more, base_url, delay_ms, request_params), // always 6 elements!
                    ))
                }
                Err(e) => {
                    warn!(
                        error = %e,
                        topic = %params.topic,
                        "Failed to retrieve historical message batch"
                    );
                    // On error: yield an SSE error event and mark stream as completed (has_more = false)
                    let error_event = format_sse_event(
                        SseEventType::Error,
                        json!({
                            "error": "Historical replay failed",
                            "message": e.to_string(),
                            "topic": params.topic
                        }),
                    );
                    let error_events = vec![Ok(web::Bytes::from(error_event))];
                    // Again, return updated state with the filtering params included
                    Some((
                        tokio_stream::iter(error_events),
                        (backend, params, false, base_url, delay_ms, request_params),
                    ))
                }
            }
        },
    )
        .flatten() // Flatten nested batch streams into one continuous SSE stream
}

/// Create a combined stream that transitions from historical to live messages
pub async fn create_historical_then_live_stream(
    topic: String,
    backend: Arc<dyn NotificationBackend>,
    from_sequence: Option<u64>,
    from_date: Option<DateTime<Utc>>,
    shutdown: web::Data<CancellationToken>,
    request_params: Arc<std::collections::HashMap<String, String>>,
) -> Result<HttpResponse> {
    let watch_config = Settings::get_global_watch_settings();
    let app_settings = Settings::get_global_application_settings();

    // Create historical replay stream
    let historical_stream = create_historical_replay_stream(
        topic.clone(),
        backend.clone(),
        from_sequence,
        from_date,
        request_params.clone(),
    );

    // Create control events for replay lifecycle
    let start_event = format_sse_event(
        SseEventType::ReplayControl,
        json!({
            "type": "replay_started",
            "topic": topic,
            "from_sequence": from_sequence,
            "from_date": from_date,
            "batch_size": watch_config.replay_batch_size,
            "timestamp": chrono::Utc::now().to_rfc3339()
        }),
    );

    let completion_event = format_sse_event(
        SseEventType::ReplayControl,
        json!({
            "type": "replay_completed",
            "topic": topic,
            "timestamp": chrono::Utc::now().to_rfc3339()
        }),
    );

    // Create live subscription stream
    // Create live subscription stream with filtering
    let notification_stream = backend.subscribe_to_topic(&topic).await?;
    let request_params_clone = request_params.clone();
    let filtered_stream = futures_util::StreamExt::filter_map(
        notification_stream,
        move |message: NotificationMessage| {
            super::live::filter_notification_message(message, request_params_clone.clone())
        },
    );

    let live_notification_sse_stream = super::live::create_live_notification_stream(
        filtered_stream,
        app_settings.base_url.clone(),
        watch_config.concurrent_notification_processing,
    );

    // Create heartbeat stream
    let heartbeat_stream =
        create_heartbeat_stream(topic.clone(), watch_config.sse_heartbeat_interval_sec);

    // Combine streams with proper event sequencing
    let combined_notification_stream = FuturesStreamExt::chain(
        FuturesStreamExt::chain(
            tokio_stream::once(Ok::<_, actix_web::Error>(web::Bytes::from(start_event))),
            historical_stream,
        ),
        FuturesStreamExt::chain(
            tokio_stream::once(Ok::<_, actix_web::Error>(web::Bytes::from(
                completion_event,
            ))),
            live_notification_sse_stream,
        ),
    );

    // Merge with heartbeat stream
    let merged_stream = TokioStreamExt::merge(combined_notification_stream, heartbeat_stream);

    // Apply graceful shutdown
    let stream_with_closing = apply_graceful_shutdown(merged_stream, shutdown.get_ref().clone());

    tracing::info!(
        topic = %topic,
        from_sequence = ?from_sequence,
        from_date = ?from_date,
        batch_size = watch_config.replay_batch_size,
        "Created combined historical-then-live SSE stream with proper event types"
    );

    Ok(create_sse_response(stream_with_closing))
}

/// Create a replay-only stream (historical messages then close)
///
/// This function creates a stream that replays historical messages and then
/// terminates the connection, unlike the watch endpoint which transitions to live streaming.
pub async fn create_replay_only_stream(
    topic: String,
    backend: Arc<dyn NotificationBackend>,
    from_sequence: Option<u64>,
    from_date: Option<DateTime<Utc>>,
    shutdown: web::Data<CancellationToken>,
    request_params: Arc<std::collections::HashMap<String, String>>,
) -> Result<HttpResponse> {
    let watch_config = Settings::get_global_watch_settings();

    // Create historical replay stream
    let historical_stream = create_historical_replay_stream(
        topic.clone(),
        backend.clone(),
        from_sequence,
        from_date,
        request_params.clone(),
    );

    // Create control events for replay lifecycle
    let start_event = format_sse_event(
        SseEventType::ReplayControl,
        json!({
            "type": "replay_started",
            "topic": topic,
            "from_sequence": from_sequence,
            "from_date": from_date,
            "batch_size": watch_config.replay_batch_size,
            "timestamp": Utc::now().to_rfc3339()
        }),
    );

    let completion_event = format_sse_event(
        SseEventType::ReplayControl,
        json!({
            "type": "replay_completed",
            "topic": topic,
            "timestamp": chrono::Utc::now().to_rfc3339()
        }),
    );

    // Chain: start -> historical -> completion
    let replay_stream = FuturesStreamExt::chain(
        FuturesStreamExt::chain(
            tokio_stream::once(Ok::<_, actix_web::Error>(web::Bytes::from(start_event))),
            historical_stream,
        ),
        tokio_stream::once(Ok::<_, actix_web::Error>(web::Bytes::from(
            completion_event,
        ))),
    );

    // Apply shutdown handling
    let stream_with_closing = apply_graceful_shutdown(replay_stream, shutdown.get_ref().clone());

    tracing::info!(
        topic = %topic,
        from_sequence = ?from_sequence,
        from_date = ?from_date,
        batch_size = watch_config.replay_batch_size,
        "Created replay-only SSE stream"
    );

    // Use existing helper for response creation
    Ok(create_sse_response(stream_with_closing))
}
