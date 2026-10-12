// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Bounded, non-streaming replay.
//!
//! [`collect_replay`] runs a replay request through the same validation as
//! `POST /api/v1/replay` and drains the same frame stream into a `Vec` of
//! CloudEvents. It is meant for callers that need one finite answer rather
//! than an SSE connection. It records no SSE connection or delivery metrics.

use crate::auth::User;
use crate::auth::stream_access::{StreamAccessError, StreamOperation, check_stream_access};
use crate::configuration::Settings;
use crate::handlers::{
    RequestParseError, StreamingRequestProcessor, ValidationConfig, parse_and_validate_request,
};
use crate::notification::wildcard_matcher::prepare_spatial_filter;
use crate::notification_backend::NotificationBackend;
use crate::routes::streaming::check_known_event_type;
use crate::sse::helpers::SseFrameKind;
use crate::sse::replay::{create_historical_replay_stream, resolve_end_sequence};
use crate::sse::types::{ControlEvent, StreamFrame};
use futures_util::StreamExt;
use serde_json::Value;
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;

/// Identity of the caller, checked against the stream's auth rules.
#[derive(Debug, Clone, Copy)]
pub struct ReplayAccess<'a> {
    /// Whether authentication is enabled on this server.
    pub auth_enabled: bool,
    /// The authenticated user, if any.
    pub user: Option<&'a User>,
    /// Admin role mapping from the auth settings.
    pub admin_roles: Option<&'a HashMap<String, Vec<String>>>,
}

/// Hard bounds for one collected replay.
#[derive(Debug, Clone, Copy)]
pub struct CollectLimits {
    /// Most notifications returned. The stream's own replay limit still
    /// applies when it is lower.
    pub max_results: NonZeroUsize,
    /// Deadline for the whole collection, backend calls included.
    pub timeout: Duration,
}

/// Notifications gathered by [`collect_replay`], oldest first.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct CollectedReplay {
    /// Event type the request resolved to.
    pub event_type: String,
    /// CloudEvents in their JSON form, as sent in SSE `data:` lines.
    pub events: Vec<Value>,
    /// True when more matching notifications existed past the limit.
    pub truncated: bool,
    /// Matching notifications that could not be rendered as CloudEvents.
    pub render_failures: usize,
}

/// Why a replay could not be collected.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CollectReplayError {
    /// The body is not a well-formed replay request.
    #[error("invalid request: {0}")]
    Parse(#[from] RequestParseError),
    /// Strict mode rejected an event type missing from the schema.
    #[error("unknown event type '{event_type}'")]
    UnknownEventType {
        event_type: String,
        /// Configured event types, sorted.
        configured: Vec<String>,
    },
    /// The caller may not read this stream.
    #[error(transparent)]
    Access(#[from] StreamAccessError),
    /// The stream needs an auth plugin this path cannot evaluate.
    #[error("stream '{event_type}' requires an auth plugin that is not supported here")]
    UnsupportedAuthPlugin { event_type: String },
    /// Identifier, start, or end point validation failed.
    #[error("invalid request: {0}")]
    Validation(String),
    /// The backend failed while resolving or reading history.
    #[error("replay failed: {0}")]
    Backend(String),
    /// The collection did not finish within [`CollectLimits::timeout`].
    #[error("replay did not finish within {0:?}")]
    Timeout(Duration),
}

/// Validate a replay request body and collect its notifications.
///
/// Validation runs in the same order as the REST endpoint: parse, strict
/// event type check, stream access, then identifier and replay parameter
/// checks. Streams with auth plugins (for example ECPDS) are refused for
/// non-admin users because the plugin check needs an HTTP request.
pub async fn collect_replay(
    body: &[u8],
    backend: Arc<dyn NotificationBackend>,
    access: ReplayAccess<'_>,
    limits: CollectLimits,
    request_id: &str,
) -> Result<CollectedReplay, CollectReplayError> {
    let request = parse_and_validate_request(body)?;
    check_known_event_type(&request.event_type).map_err(|configured| {
        CollectReplayError::UnknownEventType {
            event_type: request.event_type.clone(),
            configured,
        }
    })?;
    check_stream_access(
        access.auth_enabled,
        access.user,
        access.admin_roles,
        &request.event_type,
        StreamOperation::Read,
    )?;
    let context =
        StreamingRequestProcessor::process_request(&request, ValidationConfig::for_replay())
            .map_err(|error| CollectReplayError::Validation(error.to_string()))?;
    reject_auth_plugins(access, &context.event_type)?;

    let max_allowed =
        Settings::replay_limit_for_event_type(&context.event_type).min(limits.max_results.get());
    let drain = async {
        let end_sequence = resolve_end_sequence(backend.as_ref(), &context.topic, context.end_at)
            .await
            .map_err(|error| CollectReplayError::Backend(error.to_string()))?;
        let prepared_spatial_filter =
            Arc::new(prepare_spatial_filter(&context.canonicalized_params));
        let frames = create_historical_replay_stream(
            context.topic.clone(),
            backend,
            context.start_at,
            end_sequence,
            Arc::new(context.canonicalized_params.clone()),
            Arc::new(context.identifier_constraints.clone()),
            prepared_spatial_filter,
            request_id.to_string(),
            max_allowed,
        );
        drain_frames(frames, context.event_type.clone()).await
    };
    tokio::time::timeout(limits.timeout, drain)
        .await
        .map_err(|_| CollectReplayError::Timeout(limits.timeout))?
}

/// Refuse streams whose auth lists plugins unless the caller is an admin,
/// matching the admin bypass of the REST plugin checks.
fn reject_auth_plugins(
    access: ReplayAccess<'_>,
    event_type: &str,
) -> Result<(), CollectReplayError> {
    if !access.auth_enabled {
        return Ok(());
    }
    let has_plugins = Settings::get_global_notification_schema()
        .as_ref()
        .and_then(|schemas| schemas.get(event_type))
        .and_then(|schema| schema.auth.as_ref())
        .and_then(|auth| auth.plugins.as_ref())
        .is_some_and(|plugins| !plugins.is_empty());
    let is_admin = access
        .user
        .zip(access.admin_roles)
        .is_some_and(|(user, admin_roles)| user.is_admin(admin_roles));
    if has_plugins && !is_admin {
        return Err(CollectReplayError::UnsupportedAuthPlugin {
            event_type: event_type.to_string(),
        });
    }
    Ok(())
}

async fn drain_frames(
    frames: impl tokio_stream::Stream<Item = StreamFrame>,
    event_type: String,
) -> Result<CollectedReplay, CollectReplayError> {
    let mut collected = CollectedReplay {
        event_type,
        events: Vec::new(),
        truncated: false,
        render_failures: 0,
    };
    let mut frames = std::pin::pin!(frames);
    while let Some(frame) = frames.next().await {
        match frame {
            StreamFrame::Rendered {
                bytes,
                kind: SseFrameKind::Notification,
            } => match sse_data_json(&bytes) {
                Some(event) => collected.events.push(event),
                None => collected.render_failures += 1,
            },
            StreamFrame::Rendered {
                kind: SseFrameKind::StreamError,
                ..
            }
            | StreamFrame::Error { .. } => collected.render_failures += 1,
            StreamFrame::Control(ControlEvent::ReplayLimitReached { .. }) => {
                collected.truncated = true;
            }
            StreamFrame::ReplayFailed { message, .. } => {
                return Err(CollectReplayError::Backend(message));
            }
            _ => {}
        }
    }
    Ok(collected)
}

/// Extract the JSON payload of one rendered SSE event.
///
/// Rendered events are a single `event:` line and a single `data:` line:
/// - valid: `event: replay\ndata: {"id":"1"}\n\n` yields `{"id":"1"}`
/// - invalid: `event: replay\n\n` (no data line) yields `None`
fn sse_data_json(bytes: &[u8]) -> Option<Value> {
    let text = std::str::from_utf8(bytes).ok()?;
    let data = text.lines().find_map(|line| line.strip_prefix("data: "))?;
    serde_json::from_str(data).ok()
}

#[cfg(test)]
mod tests {
    use super::sse_data_json;
    use serde_json::json;

    #[test]
    fn sse_data_json_reads_the_data_line() {
        let bytes = b"event: replay\ndata: {\"id\":\"1\"}\n\n";
        assert_eq!(sse_data_json(bytes), Some(json!({"id": "1"})));
    }

    #[test]
    fn sse_data_json_rejects_missing_or_malformed_data() {
        assert_eq!(sse_data_json(b"event: replay\n\n"), None);
        assert_eq!(sse_data_json(b"event: replay\ndata: {oops\n\n"), None);
    }
}
