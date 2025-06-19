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
use crate::notification_backend::NotificationBackend;

/// Create a live notification stream from a backend subscription
pub(crate) fn create_live_notification_stream(
    notification_stream: impl tokio_stream::Stream<
        Item = crate::notification_backend::NotificationMessage,
    > + Send
    + 'static,
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

/// Create SSE stream for watch endpoint using tokio_stream
pub async fn create_watch_sse_stream(
    topic: String,
    backend: Arc<dyn NotificationBackend>,
    shutdown: web::Data<CancellationToken>,
) -> Result<HttpResponse> {
    let app_settings = Settings::get_global_application_settings();
    let watch_config = Settings::get_global_watch_settings();

    // Subscribe to the topic for real-time notifications
    let notification_stream = backend.subscribe_to_topic(&topic).await?;

    // Create heartbeat stream
    let heartbeat_stream =
        create_heartbeat_stream(topic.clone(), watch_config.sse_heartbeat_interval_sec);

    // Create live notification stream
    let notification_sse_stream = create_live_notification_stream(
        notification_stream,
        app_settings.base_url.clone(),
        watch_config.concurrent_notification_processing,
    );

    // Send initial connection established event
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

    // Merge streams
    let merged_stream = TokioStreamExt::merge(
        FuturesStreamExt::chain(initial_stream, notification_sse_stream),
        heartbeat_stream,
    );

    // Apply graceful shutdown
    let stream_with_closing = apply_graceful_shutdown(merged_stream, shutdown.get_ref().clone());

    tracing::info!(
        timeout_seconds = connection_timeout.as_secs(),
        concurrent_limit = watch_config.concurrent_notification_processing,
        "SSE stream created with graceful-shutdown support"
    );

    Ok(create_sse_response(stream_with_closing))
}
