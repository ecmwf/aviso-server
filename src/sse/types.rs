// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! SSE event types and formatting utilities

use crate::notification_backend::NotificationMessage;
use chrono::{DateTime, Utc};
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

#[derive(Debug, Clone)]
pub enum DeliveryKind {
    Live,
    Replay,
}

#[derive(Debug, Clone)]
pub enum CloseReason {
    ServerShutdown,
    MaxDurationReached,
    EndOfStream,
}

#[derive(Debug, Clone)]
pub enum ControlEvent {
    ConnectionEstablished {
        topic: String,
        timestamp: DateTime<Utc>,
        connection_will_close_in_seconds: u64,
    },
    ReplayStarted {
        topic: String,
        from_sequence: Option<u64>,
        from_date: Option<DateTime<Utc>>,
        batch_size: usize,
        timestamp: DateTime<Utc>,
    },
    ReplayCompleted {
        topic: String,
        timestamp: DateTime<Utc>,
    },
    ReplayLimitReached {
        topic: String,
        max_allowed: usize,
        timestamp: DateTime<Utc>,
    },
}

#[derive(Debug, Clone)]
pub enum StreamFrame {
    Notification {
        notification: NotificationMessage,
        kind: DeliveryKind,
    },
    Control(ControlEvent),
    Heartbeat {
        topic: String,
        timestamp: DateTime<Utc>,
    },
    Error {
        topic: String,
        message: String,
    },
    Close {
        topic: String,
        reason: CloseReason,
        timestamp: DateTime<Utc>,
    },
}
