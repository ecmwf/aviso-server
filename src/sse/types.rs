//! SSE event types and formatting utilities

use serde_json::Value;

/// SSE event types for different message categories
#[derive(Debug, Clone)]
pub enum SseEventType {
    /// Real-time notification as CloudEvent
    LiveNotification,
    /// Historical replay notification as CloudEvent
    ReplayNotification,
    /// Periodic heartbeat to keep connection alive
    Heartbeat,
    /// Connection closing notification
    ConnectionClosing,
    /// Error notification
    Error,
    /// Replay control events (start, complete, transition)
    ReplayControl,
}

impl SseEventType {
    /// Get the SSE event type string
    pub fn as_str(&self) -> &'static str {
        match self {
            SseEventType::LiveNotification => "live-notification",
            SseEventType::ReplayNotification => "replay",
            SseEventType::Heartbeat => "heartbeat",
            SseEventType::ConnectionClosing => "connection-closing",
            SseEventType::Error => "error",
            SseEventType::ReplayControl => "replay-control",
        }
    }
}

/// Format data as an SSE event
///
/// Creates properly formatted SSE event strings according to the
/// Server-Sent Events specification.
pub fn format_sse_event(event_type: SseEventType, data: Value) -> String {
    format!("event: {}\ndata: {}\n\n", event_type.as_str(), data)
}
