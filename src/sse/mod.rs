// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

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
