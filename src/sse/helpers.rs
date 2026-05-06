// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! SSE utility functions and common helpers

use actix_web::{HttpResponse, web};
use chrono::{DateTime, SecondsFormat, Utc};
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
use crate::telemetry::{SERVICE_NAME, SERVICE_VERSION};

fn format_stream_timestamp(timestamp: DateTime<Utc>) -> String {
    timestamp.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Convert a notification message to an SSE event
pub fn notification_to_sse_event(
    notification: &crate::notification_backend::NotificationMessage,
    base_url: &str,
    event_type: SseEventType,
    request_id: &str,
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
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_name = "stream.sse.cloudevent.creation.failed",
                error = %e,
                topic = %display_topic,
                sequence = notification.sequence,
                request_id = %request_id,
                "Failed to create CloudEvent from notification"
            );

            // CloudEvent creation failures bypass StreamFrame::Error and emit
            // an `event: error` directly here. Carrying request_id keeps the
            // payload self-contained for incident reports.
            let error_event = format_sse_event(
                SseEventType::Error,
                json!({
                    "error": "CloudEvent creation failed",
                    "message": e.to_string(),
                    "topic": display_topic,
                    "sequence": notification.sequence,
                    "request_id": request_id,
                }),
            );

            Ok(web::Bytes::from(error_event))
        }
    }
}

/// Render an internal stream frame into SSE wire bytes.
///
/// This is the single formatting boundary between typed stream state and
/// event-stream text payloads sent to clients. `request_id` is propagated
/// only for the per-message CloudEvent-failure error path; control events
/// and lifecycle frames carry their own copy inside the variant.
pub fn frame_to_sse_bytes(
    frame: StreamFrame,
    base_url: &str,
    request_id: &str,
) -> Result<web::Bytes, actix_web::Error> {
    match frame {
        StreamFrame::Notification { notification, kind } => {
            let event_type = match kind {
                DeliveryKind::Live => SseEventType::LiveNotification,
                DeliveryKind::Replay => SseEventType::ReplayNotification,
            };
            notification_to_sse_event(&notification, base_url, event_type, request_id)
        }
        StreamFrame::Control(control) => match control {
            ControlEvent::ConnectionEstablished {
                topic,
                timestamp,
                connection_will_close_in_seconds,
                request_id,
            } => Ok(web::Bytes::from(format_sse_event(
                SseEventType::LiveNotification,
                json!({
                    "type": "connection_established",
                    "topic": decode_subject_for_display(&topic),
                    "timestamp": format_stream_timestamp(timestamp),
                    "connection_will_close_in_seconds": connection_will_close_in_seconds,
                    "request_id": request_id,
                }),
            ))),
            ControlEvent::ReplayStarted {
                topic,
                from_sequence,
                from_date,
                batch_size,
                timestamp,
                request_id,
            } => Ok(web::Bytes::from(format_sse_event(
                SseEventType::ReplayControl,
                json!({
                    "type": "replay_started",
                    "topic": decode_subject_for_display(&topic),
                    "from_sequence": from_sequence,
                    "from_date": from_date.map(format_stream_timestamp),
                    "batch_size": batch_size,
                    "timestamp": format_stream_timestamp(timestamp),
                    "request_id": request_id,
                }),
            ))),
            ControlEvent::ReplayCompleted { topic, timestamp } => {
                Ok(web::Bytes::from(format_sse_event(
                    SseEventType::ReplayControl,
                    json!({
                        "type": "replay_completed",
                        "topic": decode_subject_for_display(&topic),
                        "timestamp": format_stream_timestamp(timestamp)
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
                    "timestamp": format_stream_timestamp(timestamp)
                }),
            ))),
        },
        StreamFrame::Heartbeat { topic, timestamp } => Ok(web::Bytes::from(format_sse_event(
            SseEventType::Heartbeat,
            json!({
                "timestamp": format_stream_timestamp(timestamp),
                "topic": decode_subject_for_display(&topic)
            }),
        ))),
        StreamFrame::Error {
            topic,
            message,
            request_id,
        } => Ok(web::Bytes::from(format_sse_event(
            SseEventType::Error,
            json!({
                "error": "stream_processing_failed",
                "message": message,
                "topic": decode_subject_for_display(&topic),
                "request_id": request_id,
            }),
        ))),
        StreamFrame::Close {
            topic,
            reason,
            timestamp,
            request_id,
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
                    "timestamp": format_stream_timestamp(timestamp),
                    "message": message,
                    "topic": decode_subject_for_display(&topic),
                    "request_id": request_id,
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
    request_id: String,
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

    // The terminal Close frame must be CONSTRUCTED at yield time, not at
    // stream-build time. tokio_stream::once(value) takes an already-evaluated
    // value, so a `tokio_stream::once({ block })` here would capture
    // close_code (always 0 at construction) and the timestamp at function
    // entry, breaking the ServerShutdown/MaxDurationReached close reasons
    // and producing a stream-start timestamp on every close. Using
    // futures_util::stream::once(future) defers evaluation: the async block
    // is polled only after `graceful` finishes, so close_code reflects which
    // branch of stop_future actually completed and chrono::Utc::now() reads
    // wall-clock at the moment of close.
    let close_code_for_end = close_code.clone();
    futures_util::StreamExt::chain(
        graceful,
        futures_util::stream::once(async move {
            let reason = match close_code_for_end.load(Ordering::SeqCst) {
                1 => CloseReason::ServerShutdown,
                2 => CloseReason::MaxDurationReached,
                _ => CloseReason::EndOfStream,
            };

            StreamFrame::Close {
                topic,
                reason,
                timestamp: chrono::Utc::now(),
                request_id,
            }
        }),
    )
}

/// Create a standardized SSE HttpResponse with proper headers.
///
/// When a `SseConnectionGuard` is provided, it is held alive for the
/// lifetime of the streaming body so active-connection gauges stay accurate.
pub fn create_sse_response<S>(
    stream: S,
    guard: Option<crate::metrics::SseConnectionGuard>,
) -> HttpResponse
where
    S: tokio_stream::Stream<Item = Result<web::Bytes, actix_web::Error>> + 'static,
{
    let mut builder = HttpResponse::Ok();
    builder
        .content_type("text/event-stream")
        .insert_header(("Cache-Control", "no-cache"))
        .insert_header(("Connection", "keep-alive"))
        .insert_header(("Access-Control-Allow-Origin", "*"))
        .insert_header(("X-Accel-Buffering", "no"));

    match guard {
        Some(g) => {
            let pinned = Box::pin(stream);
            builder.streaming(crate::metrics::GuardedSseStream::new(pinned, g))
        }
        None => builder.streaming(stream),
    }
}

#[cfg(test)]
mod tests {
    use super::frame_to_sse_bytes;
    use crate::sse::types::{CloseReason, ControlEvent, StreamFrame};
    use chrono::{DateTime, Utc};

    const TEST_REQUEST_ID: &str = "abcd1234-0000-4000-8000-aaaaaaaaaaaa";

    #[test]
    fn replay_started_timestamps_are_emitted_as_clean_utc_seconds() {
        let control_timestamp = DateTime::parse_from_rfc3339("2026-02-25T18:58:23.710810413+00:00")
            .expect("test timestamp should parse")
            .with_timezone(&Utc);
        let from_date = DateTime::parse_from_rfc3339("2026-02-25T17:01:02.999999999+00:00")
            .expect("test from_date should parse")
            .with_timezone(&Utc);

        let bytes = frame_to_sse_bytes(
            StreamFrame::Control(ControlEvent::ReplayStarted {
                topic: "polygon.*.1200".to_string(),
                from_sequence: Some(0),
                from_date: Some(from_date),
                batch_size: 100,
                timestamp: control_timestamp,
                request_id: TEST_REQUEST_ID.to_string(),
            }),
            "http://localhost",
            TEST_REQUEST_ID,
        )
        .expect("frame rendering should succeed");
        let text = String::from_utf8(bytes.to_vec()).expect("sse bytes should be valid utf-8");

        assert!(text.contains(r#""timestamp":"2026-02-25T18:58:23Z""#));
        assert!(text.contains(r#""from_date":"2026-02-25T17:01:02Z""#));
    }

    #[test]
    fn connection_established_event_carries_request_id() {
        let bytes = frame_to_sse_bytes(
            StreamFrame::Control(ControlEvent::ConnectionEstablished {
                topic: "test.topic".to_string(),
                timestamp: Utc::now(),
                connection_will_close_in_seconds: 3600,
                request_id: TEST_REQUEST_ID.to_string(),
            }),
            "http://localhost",
            TEST_REQUEST_ID,
        )
        .expect("frame rendering should succeed");
        let text = String::from_utf8(bytes.to_vec()).expect("sse bytes should be valid utf-8");

        assert!(text.contains(&format!(r#""request_id":"{TEST_REQUEST_ID}""#)));
        assert!(text.contains(r#""type":"connection_established""#));
    }

    #[test]
    fn replay_started_event_carries_request_id() {
        let bytes = frame_to_sse_bytes(
            StreamFrame::Control(ControlEvent::ReplayStarted {
                topic: "test.topic".to_string(),
                from_sequence: Some(42),
                from_date: None,
                batch_size: 10,
                timestamp: Utc::now(),
                request_id: TEST_REQUEST_ID.to_string(),
            }),
            "http://localhost",
            TEST_REQUEST_ID,
        )
        .expect("frame rendering should succeed");
        let text = String::from_utf8(bytes.to_vec()).expect("sse bytes should be valid utf-8");

        assert!(text.contains(&format!(r#""request_id":"{TEST_REQUEST_ID}""#)));
        assert!(text.contains(r#""type":"replay_started""#));
    }

    #[test]
    fn error_frame_carries_request_id() {
        let bytes = frame_to_sse_bytes(
            StreamFrame::Error {
                topic: "test.topic".to_string(),
                message: "backend unavailable".to_string(),
                request_id: TEST_REQUEST_ID.to_string(),
            },
            "http://localhost",
            TEST_REQUEST_ID,
        )
        .expect("frame rendering should succeed");
        let text = String::from_utf8(bytes.to_vec()).expect("sse bytes should be valid utf-8");

        assert!(text.contains(r#"event: error"#));
        assert!(text.contains(&format!(r#""request_id":"{TEST_REQUEST_ID}""#)));
        assert!(text.contains(r#""error":"stream_processing_failed""#));
    }

    #[test]
    fn close_frame_carries_request_id() {
        let bytes = frame_to_sse_bytes(
            StreamFrame::Close {
                topic: "test.topic".to_string(),
                reason: CloseReason::MaxDurationReached,
                timestamp: Utc::now(),
                request_id: TEST_REQUEST_ID.to_string(),
            },
            "http://localhost",
            TEST_REQUEST_ID,
        )
        .expect("frame rendering should succeed");
        let text = String::from_utf8(bytes.to_vec()).expect("sse bytes should be valid utf-8");

        assert!(text.contains(r#"event: connection-closing"#));
        assert!(text.contains(&format!(r#""request_id":"{TEST_REQUEST_ID}""#)));
        assert!(text.contains(r#""reason":"max_duration_reached""#));
    }

    // Pin the deferred-evaluation contract of apply_stream_lifecycle's
    // synthesized close frame. With the buggy `tokio_stream::once(value)`
    // form the close frame's reason was captured at stream construction
    // (always 0 = EndOfStream); the fix uses
    // `futures_util::stream::once(future)` so close_code is read at yield
    // time and reflects which branch of stop_future actually completed.
    // Pre-cancelling the shutdown token gives a deterministic
    // ServerShutdown verdict that fails on the buggy code and passes on
    // the fixed code.
    #[tokio::test]
    async fn close_frame_reflects_actual_close_reason_via_apply_stream_lifecycle() {
        use super::apply_stream_lifecycle;
        use std::time::Duration;
        use tokio_stream::StreamExt as _;

        let inner: futures_util::stream::Pending<StreamFrame> = futures_util::stream::pending();
        let token = tokio_util::sync::CancellationToken::new();
        token.cancel();

        let stream = apply_stream_lifecycle(
            inner,
            "test.topic".to_string(),
            token,
            Some(Duration::from_secs(60)),
            "req-shutdown".to_string(),
        );
        tokio::pin!(stream);

        let frame = stream.next().await.expect("close frame should be yielded");
        match frame {
            StreamFrame::Close {
                reason,
                request_id,
                topic,
                ..
            } => {
                assert!(
                    matches!(reason, CloseReason::ServerShutdown),
                    "close reason should reflect actual cause (ServerShutdown), got: {reason:?}"
                );
                assert_eq!(request_id, "req-shutdown");
                assert_eq!(topic, "test.topic");
            }
            other => panic!("expected StreamFrame::Close, got: {other:?}"),
        }

        assert!(
            stream.next().await.is_none(),
            "stream should terminate after the close frame"
        );
    }
}
