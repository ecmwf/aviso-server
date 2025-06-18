//! Server-Sent Events (SSE) streaming infrastructure for watch endpoint
//!
//! This module provides SSE event formatting and streaming capabilities
//! for real-time notification delivery to clients.

pub mod helpers;
pub mod live;
pub mod replay;
pub mod types;

// Re-export the main public API
pub use helpers::{create_sse_response, notification_to_sse_event};
pub use live::create_watch_sse_stream;
pub use replay::{create_historical_replay_stream, create_historical_then_live_stream};
pub use types::{SseEventType, format_sse_event};
