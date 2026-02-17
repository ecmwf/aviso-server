//! Live notification streaming functionality

use actix_web::{HttpResponse, web};
use anyhow::Result;
use futures_util::StreamExt as FuturesStreamExt;
use std::sync::Arc;
use tokio::time::Duration;
use tokio_stream::StreamExt as TokioStreamExt;
use tokio_util::sync::CancellationToken;

use super::helpers::{
    apply_stream_lifecycle, create_heartbeat_stream, create_sse_response, frame_to_sse_bytes,
};
use super::types::{ControlEvent, DeliveryKind, StreamFrame};
use crate::configuration::Settings;
use crate::notification::decode_subject_for_display;
use crate::notification_backend::{NotificationBackend, NotificationMessage};

/// Create a live notification stream from a backend subscription
pub fn create_live_notification_stream(
    notification_stream: impl tokio_stream::Stream<Item = NotificationMessage> + Send + 'static,
    _concurrent_limit: usize,
) -> impl tokio_stream::Stream<Item = StreamFrame> {
    // Preserve backend emission order. This path intentionally avoids unordered buffering.
    FuturesStreamExt::map(notification_stream, move |notification| {
        StreamFrame::Notification {
            notification,
            kind: DeliveryKind::Live,
        }
    })
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
    request_params: Arc<std::collections::HashMap<String, String>>,
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

    // Convert filtered notifications into typed live frames.
    let notification_sse_stream = create_live_notification_stream(
        filtered_stream,
        watch_config.concurrent_notification_processing,
    );

    // Send initial connection established event.
    let connection_timeout = Duration::from_secs(watch_config.connection_max_duration_sec);
    let initial_stream =
        tokio_stream::once(StreamFrame::Control(ControlEvent::ConnectionEstablished {
            topic: topic.clone(),
            timestamp: chrono::Utc::now(),
            connection_will_close_in_seconds: connection_timeout.as_secs(),
        }));

    // Create heartbeat stream
    let heartbeat_stream =
        create_heartbeat_stream(topic.clone(), watch_config.sse_heartbeat_interval_sec);

    // Merge: initial event + live notifications, then merge with heartbeat
    let merged_stream = TokioStreamExt::merge(
        FuturesStreamExt::chain(initial_stream, notification_sse_stream),
        heartbeat_stream,
    );

    // Apply lifecycle and convert typed frames to SSE bytes.
    let stream_with_lifecycle = apply_stream_lifecycle(
        merged_stream,
        topic.clone(),
        shutdown.get_ref().clone(),
        Some(connection_timeout),
    );
    let base_url = app_settings.base_url.clone();
    let byte_stream = FuturesStreamExt::map(stream_with_lifecycle, move |frame| {
        frame_to_sse_bytes(frame, &base_url)
    });

    tracing::info!(
        topic = %decode_subject_for_display(&topic),
        timeout_seconds = connection_timeout.as_secs(),
        concurrent_limit = watch_config.concurrent_notification_processing,
        "SSE stream created with graceful-shutdown support and filtering"
    );

    Ok(create_sse_response(byte_stream))
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
