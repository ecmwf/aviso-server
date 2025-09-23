//! Live notification streaming functionality

use actix_web::{HttpResponse, web};
use anyhow::Result;
use futures_util::StreamExt as FuturesStreamExt;
use serde_json::json;
use std::sync::Arc;
use tokio::time::Duration;
use tokio_stream::StreamExt as TokioStreamExt;
use tokio_util::sync::CancellationToken;

use super::helpers::{
    apply_graceful_shutdown, create_heartbeat_stream, create_sse_response,
    notification_to_sse_event,
};
use super::types::{SseEventType, format_sse_event};
use crate::configuration::Settings;
use crate::notification_backend::{NotificationBackend, NotificationMessage};

/// Create a live notification stream from a backend subscription
pub fn create_live_notification_stream(
    notification_stream: impl tokio_stream::Stream<Item = NotificationMessage> + Send + 'static,
    base_url: String,
    concurrent_limit: usize,
) -> impl tokio_stream::Stream<Item = Result<web::Bytes, actix_web::Error>> {
    FuturesStreamExt::buffer_unordered(
        FuturesStreamExt::map(notification_stream, move |notification| {
            let base_url = base_url.clone();
            async move {
                notification_to_sse_event(&notification, &base_url, SseEventType::LiveNotification)
            }
        }),
        concurrent_limit,
    )
}

/// Create a live SSE stream for the watch endpoint, applying field/spatial filtering
///
/// - Subscribes to the notification topic for real-time events
/// - Filters each notification using matches_notification_filters (param/polygon filtering)
/// - Sends connection established and heartbeat events, supports graceful shutdown
pub async fn create_watch_sse_stream(
    topic: String,
    backend: Arc<dyn NotificationBackend>,
    shutdown: web::Data<CancellationToken>,
    request_params: Arc<std::collections::HashMap<String, String>>, // NEW: filtering params
) -> Result<HttpResponse> {
    let app_settings = Settings::get_global_application_settings();
    let watch_config = Settings::get_global_watch_settings();

    // Subscribe to the topic for real-time notifications
    let notification_stream = backend.subscribe_to_topic(&topic).await?;

    let request_params_clone = request_params.clone();
    let filtered_stream = futures_util::StreamExt::filter_map(
        notification_stream,
        move |message: NotificationMessage| {
            filter_notification_message(message, request_params_clone.clone())
        },
    );

    // Convert filtered notifications into SSE events (Cloudevents)
    let notification_sse_stream = create_live_notification_stream(
        filtered_stream,
        app_settings.base_url.clone(),
        watch_config.concurrent_notification_processing,
    );

    // Send initial connection established event (one-shot SSE)
    let connection_timeout = Duration::from_secs(watch_config.connection_max_duration_sec);
    let initial_event = format_sse_event(
        SseEventType::LiveNotification,
        json!({
            "type": "connection_established",
            "topic": topic,
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "connection_will_close_in_seconds": connection_timeout.as_secs()
        }),
    );
    let initial_stream =
        tokio_stream::once(Ok::<_, actix_web::Error>(web::Bytes::from(initial_event)));

    // Create heartbeat stream
    let heartbeat_stream =
        create_heartbeat_stream(topic.clone(), watch_config.sse_heartbeat_interval_sec);

    // Merge: initial event + live notifications, then merge with heartbeat
    let merged_stream = TokioStreamExt::merge(
        FuturesStreamExt::chain(initial_stream, notification_sse_stream),
        heartbeat_stream,
    );

    // Apply graceful shutdown
    let stream_with_closing = apply_graceful_shutdown(merged_stream, shutdown.get_ref().clone());

    tracing::info!(
        timeout_seconds = connection_timeout.as_secs(),
        concurrent_limit = watch_config.concurrent_notification_processing,
        "SSE stream created with graceful-shutdown support and filtering"
    );

    Ok(create_sse_response(stream_with_closing))
}

pub async fn filter_notification_message(
    message: NotificationMessage,
    request_params: Arc<std::collections::HashMap<String, String>>,
) -> Option<NotificationMessage> {
    let result = crate::notification::wildcard_matcher::matches_notification_filters(
        &request_params,
        message.metadata.as_ref(),
        &message.payload,
    );
    tracing::debug!(
        filter_result = result,
        request_params = ?*request_params,
        message_metadata = ?message.metadata,
        message_payload = %message.payload,
        "Live filter decision"
    );
    if result { Some(message) } else { None }
}
