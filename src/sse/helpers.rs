//! SSE utility functions and common helpers

use actix_web::{HttpResponse, web};
use futures_util::stream::unfold;
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::Level;
use tracing::{debug, warn};

use super::types::{
    CloseReason, ControlEvent, DeliveryKind, SseEventType, StreamFrame, format_sse_event,
};
use crate::cloudevents::create_cloud_event_from_notification;
use crate::notification::decode_subject_for_display;

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

            if tracing::enabled!(Level::DEBUG) {
                let display_topic = decode_subject_for_display(&notification.topic);
                debug!(
                    topic = %display_topic,
                    sequence = notification.sequence,
                    event_type = %event_type.as_str(),
                    "Converted notification to SSE event"
                );
            }

            Ok(web::Bytes::from(sse_event))
        }
        Err(e) => {
            let display_topic = decode_subject_for_display(&notification.topic);
            warn!(
                error = %e,
                topic = %display_topic,
                sequence = notification.sequence,
                "Failed to create CloudEvent from notification"
            );

            let error_event = format_sse_event(
                SseEventType::Error,
                json!({
                    "error": "CloudEvent creation failed",
                    "message": e.to_string(),
                    "topic": display_topic,
                    "sequence": notification.sequence
                }),
            );

            Ok(web::Bytes::from(error_event))
        }
    }
}

/// Render an internal stream frame into SSE wire bytes.
///
/// This is the single formatting boundary between typed stream state and
/// event-stream text payloads sent to clients.
pub fn frame_to_sse_bytes(
    frame: StreamFrame,
    base_url: &str,
) -> Result<web::Bytes, actix_web::Error> {
    match frame {
        StreamFrame::Notification { notification, kind } => {
            let event_type = match kind {
                DeliveryKind::Live => SseEventType::LiveNotification,
                DeliveryKind::Replay => SseEventType::ReplayNotification,
            };
            notification_to_sse_event(&notification, base_url, event_type)
        }
        StreamFrame::Control(control) => match control {
            ControlEvent::ConnectionEstablished {
                topic,
                timestamp,
                connection_will_close_in_seconds,
            } => Ok(web::Bytes::from(format_sse_event(
                SseEventType::LiveNotification,
                json!({
                    "type": "connection_established",
                    "topic": decode_subject_for_display(&topic),
                    "timestamp": timestamp.to_rfc3339(),
                    "connection_will_close_in_seconds": connection_will_close_in_seconds
                }),
            ))),
            ControlEvent::ReplayStarted {
                topic,
                from_sequence,
                from_date,
                batch_size,
                timestamp,
            } => Ok(web::Bytes::from(format_sse_event(
                SseEventType::ReplayControl,
                json!({
                    "type": "replay_started",
                    "topic": decode_subject_for_display(&topic),
                    "from_sequence": from_sequence,
                    "from_date": from_date,
                    "batch_size": batch_size,
                    "timestamp": timestamp.to_rfc3339()
                }),
            ))),
            ControlEvent::ReplayCompleted { topic, timestamp } => {
                Ok(web::Bytes::from(format_sse_event(
                    SseEventType::ReplayControl,
                    json!({
                        "type": "replay_completed",
                        "topic": decode_subject_for_display(&topic),
                        "timestamp": timestamp.to_rfc3339()
                    }),
                )))
            }
            ControlEvent::ReplayLimitReached {
                topic,
                max_allowed,
                timestamp,
            } => Ok(web::Bytes::from(format_sse_event(
                SseEventType::ReplayControl,
                json!({
                    "type": "notification_replay_limit_reached",
                    "topic": decode_subject_for_display(&topic),
                    "max_allowed": max_allowed,
                    "message": format!(
                        "Historical replay limited to {} messages. Additional historical messages may be available but were not retrieved.",
                        max_allowed
                    ),
                    "timestamp": timestamp.to_rfc3339()
                }),
            ))),
        },
        StreamFrame::Heartbeat { topic, timestamp } => Ok(web::Bytes::from(format_sse_event(
            SseEventType::Heartbeat,
            json!({
                "timestamp": timestamp.to_rfc3339(),
                "topic": decode_subject_for_display(&topic)
            }),
        ))),
        StreamFrame::Error { topic, message } => Ok(web::Bytes::from(format_sse_event(
            SseEventType::Error,
            json!({
                "error": "stream_processing_failed",
                "message": message,
                "topic": decode_subject_for_display(&topic)
            }),
        ))),
        StreamFrame::Close {
            topic,
            reason,
            timestamp,
        } => {
            let (reason_str, message) = match reason {
                CloseReason::ServerShutdown => (
                    "server_shutdown",
                    "Server is shutting down gracefully".to_string(),
                ),
                CloseReason::MaxDurationReached => (
                    "max_duration_reached",
                    "Connection reached maximum duration".to_string(),
                ),
                CloseReason::EndOfStream => ("end_of_stream", "Stream completed".to_string()),
            };
            Ok(web::Bytes::from(format_sse_event(
                SseEventType::ConnectionClosing,
                json!({
                    "reason": reason_str,
                    "timestamp": timestamp.to_rfc3339(),
                    "message": message,
                    "topic": decode_subject_for_display(&topic)
                }),
            )))
        }
    }
}

/// Create a heartbeat stream for SSE connections
pub fn create_heartbeat_stream(
    topic: String,
    interval_seconds: u64,
) -> impl tokio_stream::Stream<Item = StreamFrame> {
    let heartbeat_interval = Duration::from_secs(interval_seconds);
    let heartbeat_interval_timer = tokio::time::interval(heartbeat_interval);

    unfold(heartbeat_interval_timer, move |mut timer| {
        let topic_clone = topic.clone();
        async move {
            timer.tick().await;
            Some((
                StreamFrame::Heartbeat {
                    topic: topic_clone,
                    timestamp: chrono::Utc::now(),
                },
                timer,
            ))
        }
    })
}

/// Apply lifecycle boundaries to a frame stream and append a terminal close frame.
///
/// Close reason precedence:
/// 1. server shutdown
/// 2. max duration reached
/// 3. natural end-of-stream
pub fn apply_stream_lifecycle<S>(
    stream: S,
    topic: String,
    shutdown_token: CancellationToken,
    max_duration: Option<Duration>,
) -> impl tokio_stream::Stream<Item = StreamFrame>
where
    S: tokio_stream::Stream<Item = StreamFrame>,
{
    let close_code = Arc::new(AtomicU8::new(0));
    let close_code_clone = close_code.clone();

    let stop_future = async move {
        match max_duration {
            Some(duration) => {
                tokio::select! {
                    _ = shutdown_token.cancelled() => {
                        tracing::debug!("SSE stream received shutdown signal");
                        close_code_clone.store(1, Ordering::SeqCst);
                    }
                    _ = tokio::time::sleep(duration) => {
                        tracing::debug!("SSE stream reached max duration");
                        close_code_clone.store(2, Ordering::SeqCst);
                    }
                }
            }
            None => {
                shutdown_token.cancelled().await;
                tracing::debug!("SSE stream received shutdown signal");
                close_code_clone.store(1, Ordering::SeqCst);
            }
        }
    };

    let graceful = futures_util::StreamExt::take_until(stream, stop_future);

    let close_code_for_end = close_code.clone();
    futures_util::StreamExt::chain(
        graceful,
        tokio_stream::once({
            let reason = match close_code_for_end.load(Ordering::SeqCst) {
                1 => CloseReason::ServerShutdown,
                2 => CloseReason::MaxDurationReached,
                _ => CloseReason::EndOfStream,
            };

            StreamFrame::Close {
                topic,
                reason,
                timestamp: chrono::Utc::now(),
            }
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
