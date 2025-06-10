//! Server-Sent Events (SSE) streaming infrastructure for watch endpoint
//!
//! This module provides SSE event formatting and streaming capabilities
//! for real-time notification delivery to clients.

use actix_web::{HttpResponse, web};
use futures_util::StreamExt as FuturesStreamExt;
use serde_json::json;
use std::sync::Arc;
use tokio::time::Duration;
use tokio_stream::StreamExt as TokioStreamExt;
use tokio_stream::wrappers::IntervalStream;

use crate::cloudevents::create_cloud_event_from_notification;
use crate::configuration::Settings;
use crate::notification_backend::NotificationBackend;
use anyhow::Result;

/// SSE event types for different message categories
#[derive(Debug, Clone)]
pub enum SseEventType {
    /// Real-time notification as CloudEvent
    LiveNotification,
    /// Periodic heartbeat to keep connection alive
    Heartbeat,
    /// Connection closing notification
    ConnectionClosing,
    /// Error notification
    Error,
}

impl SseEventType {
    /// Get the SSE event type string
    pub fn as_str(&self) -> &'static str {
        match self {
            SseEventType::LiveNotification => "live-notification",
            SseEventType::Heartbeat => "heartbeat",
            SseEventType::ConnectionClosing => "connection-closing",
            SseEventType::Error => "error",
        }
    }
}

/// Format data as an SSE event
///
/// Creates properly formatted SSE event strings according to the
/// Server-Sent Events specification.
pub fn format_sse_event(event_type: SseEventType, data: serde_json::Value) -> String {
    format!("event: {}\ndata: {}\n\n", event_type.as_str(), data)
}

/// Create SSE stream for watch endpoint using tokio_stream
///
/// Sets up a complete SSE streaming pipeline using tokio_stream combinators.
pub async fn create_watch_sse_stream(
    topic: String,
    backend: Arc<dyn NotificationBackend>,
) -> Result<HttpResponse> {
    let app_settings = Settings::get_global_application_settings();
    let watch_config = Settings::get_global_watch_settings();

    // Subscribe to the topic for real-time notifications
    let notification_stream = backend.subscribe_to_topic(&topic).await?;

    // Create heartbeat timer (skip first immediate tick)
    let heartbeat_interval = Duration::from_secs(watch_config.sse_heartbeat_interval_sec);
    let topic_clone = topic.clone();
    let mut heartbeat_interval_timer = tokio::time::interval(heartbeat_interval);
    heartbeat_interval_timer.tick().await; // Skip the immediate first tick

    let heartbeat_stream =
        FuturesStreamExt::map(IntervalStream::new(heartbeat_interval_timer), move |_| {
            let heartbeat_event = format_sse_event(
                SseEventType::Heartbeat,
                json!({
                    "timestamp": chrono::Utc::now().to_rfc3339(),
                    "topic": topic_clone.clone()
                }),
            );
            Ok::<_, actix_web::Error>(web::Bytes::from(heartbeat_event))
        });

    // Convert notifications to SSE events with configurable concurrent processing
    let base_url = app_settings.base_url.clone();
    let concurrent_limit = watch_config.concurrent_notification_processing;

    let notification_sse_stream = FuturesStreamExt::buffer_unordered(
        FuturesStreamExt::map(notification_stream, move |notification| {
            let base_url = base_url.clone();
            async move {
                match create_cloud_event_from_notification(&notification, &base_url) {
                    Ok(cloud_event) => {
                        let event_data = serde_json::to_value(&cloud_event)
                            .unwrap_or_else(|_| json!({"error": "Failed to serialize CloudEvent"}));

                        let sse_event =
                            format_sse_event(SseEventType::LiveNotification, event_data);

                        tracing::debug!(
                            topic = %notification.topic,
                            sequence = notification.sequence,
                            "Sending live notification via SSE"
                        );

                        Ok::<_, actix_web::Error>(web::Bytes::from(sse_event))
                    }
                    Err(e) => {
                        tracing::error!(
                            error = %e,
                            topic = %notification.topic,
                            "Failed to create CloudEvent from notification"
                        );

                        let error_event = format_sse_event(
                            SseEventType::Error,
                            json!({
                                "error": "CloudEvent creation failed",
                                "message": e.to_string(),
                                "topic": notification.topic
                            }),
                        );
                        Ok(web::Bytes::from(error_event))
                    }
                }
            }
        }),
        concurrent_limit,
    );

    // Send initial connection established event with timeout info
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

    // Create a stream that sends a closing message before terminating
    let stream_with_closing = FuturesStreamExt::chain(
        FuturesStreamExt::take_until(merged_stream, tokio::time::sleep(connection_timeout)),
        // Send final closing message
        tokio_stream::once({
            let closing_event = format_sse_event(
                SseEventType::ConnectionClosing,
                json!({
                    "reason": "timeout_reached",
                    "timestamp": chrono::Utc::now().to_rfc3339(),
                    "message": "Connection is closing due to timeout"
                }),
            );
            Ok::<_, actix_web::Error>(web::Bytes::from(closing_event))
        }),
    );

    tracing::info!(
        topic = %topic,
        timeout_seconds = connection_timeout.as_secs(),
        concurrent_limit = concurrent_limit,
        "SSE stream created with closing message support"
    );

    Ok(HttpResponse::Ok()
        .content_type("text/event-stream")
        .insert_header(("Cache-Control", "no-cache"))
        .insert_header(("Connection", "keep-alive"))
        .insert_header(("Access-Control-Allow-Origin", "*"))
        .insert_header(("X-Accel-Buffering", "no"))
        .streaming(stream_with_closing))
}
