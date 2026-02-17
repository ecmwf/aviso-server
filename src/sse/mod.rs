//! Server-Sent Events (SSE) streaming infrastructure.
//!
//! The SSE implementation uses a typed internal stream model:
//! - producers emit `StreamFrame` values (control, notifications, heartbeat, errors, close)
//! - helpers apply lifecycle rules (shutdown/timeout/end-of-stream)
//! - a single renderer converts frames to wire-format SSE bytes
//!
//! This keeps endpoint behavior stable while making stream lifecycle easier to reason about.

pub mod helpers;
pub mod live;
pub mod replay;
pub mod types;

// Re-export the main public API
pub use helpers::{create_sse_response, notification_to_sse_event};
pub use live::create_watch_sse_stream;
pub use replay::{
    create_historical_replay_stream, create_historical_then_live_stream, create_replay_only_stream,
};
pub use types::{SseEventType, format_sse_event};
