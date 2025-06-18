//! SSE utility functions and common helpers

use actix_web::{HttpResponse, web};
use futures_util::stream::unfold;
use serde_json::json;
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use super::types::{SseEventType, format_sse_event};
use crate::cloudevents::create_cloud_event_from_notification;

/// Convert a notification message to an SSE event
pub fn notification_to_sse_event(
    notification: &crate::notification_backend::NotificationMessage,
    base_url: &str,
    event_type: SseEventType,
) -> Result<web::Bytes, actix_web::Error> {
    match create_cloud_event_from_notification(notification, base_url) {
        Ok(cloud_event) => {
            let event_data = serde_json::to_value(&cloud_event)
                .unwrap_or_else(|_| json!({"error": "Failed to serialize CloudEvent"}));

            let sse_event = format_sse_event(event_type.clone(), event_data);

            debug!(
                topic = %notification.topic,
                sequence = notification.sequence,
                event_type = %event_type.as_str(),
                "Converted notification to SSE event"
            );

            Ok(web::Bytes::from(sse_event))
        }
        Err(e) => {
            warn!(
                error = %e,
                topic = %notification.topic,
                sequence = notification.sequence,
                "Failed to create CloudEvent from notification"
            );

            let error_event = format_sse_event(
                SseEventType::Error,
                json!({
                    "error": "CloudEvent creation failed",
                    "message": e.to_string(),
                    "topic": notification.topic,
                    "sequence": notification.sequence
                }),
            );

            Ok(web::Bytes::from(error_event))
        }
    }
}

/// Create a heartbeat stream for SSE connections
pub fn create_heartbeat_stream(
    topic: String,
    interval_seconds: u64,
) -> impl tokio_stream::Stream<Item = Result<web::Bytes, actix_web::Error>> {
    let heartbeat_interval = Duration::from_secs(interval_seconds);
    let heartbeat_interval_timer = tokio::time::interval(heartbeat_interval);

    unfold(heartbeat_interval_timer, move |mut timer| {
        let topic_clone = topic.clone();
        async move {
            timer.tick().await;
            let heartbeat_event = format_sse_event(
                SseEventType::Heartbeat,
                json!({
                    "timestamp": chrono::Utc::now().to_rfc3339(),
                    "topic": topic_clone
                }),
            );
            Some((Ok(web::Bytes::from(heartbeat_event)), timer))
        }
    })
}

/// Apply graceful shutdown handling to any stream
pub fn apply_graceful_shutdown<S>(
    stream: S,
    shutdown_token: CancellationToken,
) -> impl tokio_stream::Stream<Item = Result<web::Bytes, actix_web::Error>>
where
    S: tokio_stream::Stream<Item = Result<web::Bytes, actix_web::Error>>,
{
    let shutdown_future = {
        let token = shutdown_token.clone();
        async move {
            token.cancelled().await;
            tracing::debug!("SSE stream received shutdown signal");
        }
    };

    let graceful = futures_util::StreamExt::take_until(stream, shutdown_future);

    futures_util::StreamExt::chain(
        graceful,
        tokio_stream::once({
            let closing_event = format_sse_event(
                SseEventType::ConnectionClosing,
                json!({
                    "reason": "server_shutdown",
                    "timestamp": chrono::Utc::now().to_rfc3339(),
                    "message": "Server is shutting down gracefully"
                }),
            );
            Ok::<_, actix_web::Error>(web::Bytes::from(closing_event))
        }),
    )
}

/// Create a standardized SSE HttpResponse with proper headers
pub fn create_sse_response<S>(stream: S) -> HttpResponse
where
    S: tokio_stream::Stream<Item = Result<web::Bytes, actix_web::Error>> + 'static,
{
    HttpResponse::Ok()
        .content_type("text/event-stream")
        .insert_header(("Cache-Control", "no-cache"))
        .insert_header(("Connection", "keep-alive"))
        .insert_header(("Access-Control-Allow-Origin", "*"))
        .insert_header(("X-Accel-Buffering", "no"))
        .streaming(stream)
}
