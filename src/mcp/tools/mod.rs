// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Read-only MCP tools over the Aviso schema and notification history.
//!
//! Tools report bad arguments and denied access as tool results with
//! `isError: true`, so the model sees the reason and can correct itself.
//! Only an unknown tool name or a malformed `tools/call` envelope is a
//! protocol error.

pub mod definitions;

use crate::auth::User;
use crate::configuration::{McpSettings, Settings};
use crate::mcp::access::{mcp_can_read, mcp_replay_access};
use crate::notification_backend::NotificationBackend;
use crate::routes::schema::{event_schema_body, lookup_event_schema};
use crate::sse::collect::{CollectLimits, CollectReplayError, collect_replay};
use crate::telemetry::{SERVICE_NAME, SERVICE_VERSION};
use chrono::{Duration as ChronoDuration, SecondsFormat, Utc};
use definitions::{
    CHECK_DATA_AVAILABLE, DESCRIBE_EVENT_TYPE, HEALTH, LIST_EVENT_TYPES, REPLAY_NOTIFICATIONS,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tracing::warn;

/// Tool error text for a replay the backend could not serve.
const REPLAY_FAILED: &str = "Replay failed on the server. Try again later.";

/// Default look-back of a replay without a start point.
const DEFAULT_REPLAY_WINDOW_HOURS: i64 = 24;

/// Caller and server context for one tool call.
pub struct ToolContext<'a> {
    /// The verified caller; never an admin.
    pub user: &'a User,
    /// The `mcp` settings.
    pub settings: &'a McpSettings,
    /// Notification backend for tools that read history.
    pub backend: Arc<dyn NotificationBackend>,
}

/// A `tools/call` that could not run at all.
#[derive(Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum ToolCallError {
    /// `params` is not an object with a string `name`.
    #[error("tools/call requires params with a string name")]
    MalformedCall,
    /// No tool has this name.
    #[error("Unknown tool: {0}")]
    UnknownTool(String),
}

/// A finished tool call, in MCP `CallToolResult` form.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutput {
    /// Name of the tool that ran.
    pub name: String,
    /// The `CallToolResult` object.
    pub result: Value,
    /// Whether `result` reports a tool error.
    pub is_error: bool,
}

/// Result of `tools/list`.
pub fn list() -> Value {
    json!({"tools": definitions::all()})
}

/// Run the tool named in `params`.
pub async fn call(
    context: &ToolContext<'_>,
    params: Option<&Value>,
) -> Result<ToolOutput, ToolCallError> {
    let params = params
        .and_then(Value::as_object)
        .ok_or(ToolCallError::MalformedCall)?;
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or(ToolCallError::MalformedCall)?;
    let arguments = match params.get("arguments") {
        None | Some(Value::Null) => Value::Object(Map::new()),
        Some(arguments) => arguments.clone(),
    };
    let outcome = match name {
        HEALTH => no_arguments(arguments).map(|()| health(context)),
        LIST_EVENT_TYPES => no_arguments(arguments).map(|()| list_event_types(context)),
        DESCRIBE_EVENT_TYPE => describe_event_type(context, arguments),
        REPLAY_NOTIFICATIONS => replay_notifications(context, arguments).await,
        CHECK_DATA_AVAILABLE => check_data_available(context, arguments).await,
        other => return Err(ToolCallError::UnknownTool(other.to_string())),
    };
    let (result, is_error) = match outcome {
        Ok(structured) => (structured_result(structured), false),
        Err(message) => (error_result(&message), true),
    };
    Ok(ToolOutput {
        name: name.to_string(),
        result,
        is_error,
    })
}

fn structured_result(structured: Value) -> Value {
    json!({
        "content": [{"type": "text", "text": structured.to_string()}],
        "structuredContent": structured,
        "isError": false
    })
}

fn error_result(message: &str) -> Value {
    json!({
        "content": [{"type": "text", "text": message}],
        "isError": true
    })
}

fn no_arguments(arguments: Value) -> Result<(), String> {
    match arguments {
        Value::Object(map) if map.is_empty() => Ok(()),
        _ => Err("This tool takes no arguments".to_string()),
    }
}

fn parse_arguments<T: for<'de> Deserialize<'de>>(arguments: Value) -> Result<T, String> {
    serde_json::from_value(arguments).map_err(|error| format!("Invalid arguments: {error}"))
}

fn health(context: &ToolContext<'_>) -> Value {
    let connected = context.backend.connection_healthy();
    json!({
        "status": if connected { "ok" } else { "degraded" },
        "backend_connected": connected,
        "server_version": SERVICE_VERSION
    })
}

/// Configured event types the caller may read.
fn list_event_types(context: &ToolContext<'_>) -> Value {
    let mut event_types: Vec<&String> = Settings::get_global_notification_schema()
        .as_ref()
        .map(|schemas| {
            schemas
                .keys()
                .filter(|name| mcp_can_read(context.user, name))
                .collect()
        })
        .unwrap_or_default();
    event_types.sort();
    json!({"event_types": event_types})
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DescribeArguments {
    event_type: String,
}

/// Describe one event type. A type the caller may not read answers
/// exactly like an unknown one, so its existence is not revealed.
fn describe_event_type(context: &ToolContext<'_>, arguments: Value) -> Result<Value, String> {
    let arguments: DescribeArguments = parse_arguments(arguments)?;
    let event_type = arguments.event_type;
    if !mcp_can_read(context.user, &event_type) {
        return Err(not_found(&event_type));
    }
    let schema = lookup_event_schema(&event_type).map_err(|_| not_found(&event_type))?;
    Ok(event_schema_body(&event_type, &schema))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplayArguments {
    event_type: String,
    identifier: Map<String, Value>,
    #[serde(default)]
    from_date: Option<String>,
    #[serde(default)]
    from_id: Option<String>,
    #[serde(default)]
    to_date: Option<String>,
    #[serde(default)]
    to_id: Option<String>,
    #[serde(default)]
    max_results: Option<usize>,
}

async fn replay_notifications(
    context: &ToolContext<'_>,
    arguments: Value,
) -> Result<Value, String> {
    let arguments: ReplayArguments = parse_arguments(arguments)?;
    let from_date = match (&arguments.from_date, &arguments.from_id) {
        (None, None) => Some(default_from_date()),
        _ => arguments.from_date,
    };
    let body = json!({
        "event_type": &arguments.event_type,
        "identifier": arguments.identifier,
        "from_date": from_date,
        "from_id": arguments.from_id,
        "to_date": arguments.to_date,
        "to_id": arguments.to_id,
    });
    let max_results = arguments
        .max_results
        .unwrap_or(context.settings.max_results)
        .min(context.settings.max_results);
    let collected = run_replay(context, &arguments.event_type, &body, max_results).await?;
    Ok(json!({
        "event_type": collected.event_type,
        "count": collected.events.len(),
        "truncated": collected.truncated,
        "notifications": collected.events
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AvailabilityArguments {
    event_type: String,
    identifier: Map<String, Value>,
    #[serde(default)]
    from_date: Option<String>,
}

async fn check_data_available(
    context: &ToolContext<'_>,
    arguments: Value,
) -> Result<Value, String> {
    let arguments: AvailabilityArguments = parse_arguments(arguments)?;
    // Without a start time, sequence 1 covers the whole retained history.
    let from_id = arguments.from_date.is_none().then_some("1");
    let body = json!({
        "event_type": &arguments.event_type,
        "identifier": arguments.identifier,
        "from_date": arguments.from_date,
        "from_id": from_id,
    });
    let collected = run_replay(context, &arguments.event_type, &body, 1).await?;
    let notification = collected.events.into_iter().next();
    let first_seen = notification
        .as_ref()
        .and_then(|event| event.get("time"))
        .cloned()
        .unwrap_or(Value::Null);
    Ok(json!({
        "available": notification.is_some(),
        "first_seen": first_seen,
        "notification": notification
    }))
}

/// Answer for an event type that is unknown or that the caller may not
/// read. Both cases look the same, so the tools do not reveal which
/// event types exist.
fn not_found(event_type: &str) -> String {
    format!("Event type '{event_type}' not found")
}

async fn run_replay(
    context: &ToolContext<'_>,
    event_type: &str,
    body: &Value,
    max_results: usize,
) -> Result<crate::sse::collect::CollectedReplay, String> {
    let max_results = NonZeroUsize::new(max_results)
        .ok_or_else(|| "max_results must be at least 1".to_string())?;
    if !mcp_can_read(context.user, event_type) {
        return Err(not_found(event_type));
    }
    let limits = CollectLimits {
        max_results,
        timeout: Duration::from_millis(context.settings.request_timeout_ms),
    };
    let request_id = uuid::Uuid::new_v4().to_string();
    collect_replay(
        body.to_string().as_bytes(),
        context.backend.clone(),
        mcp_replay_access(context.user),
        limits,
        &request_id,
    )
    .await
    .map_err(|error| replay_error(event_type, error))
}

/// Text the model sees for a failed replay. Request problems keep their
/// detail so the model can fix the call. Backend failures may carry
/// server internals, so the model gets a generic message and the detail
/// goes to the log.
fn replay_error(event_type: &str, error: CollectReplayError) -> String {
    match error {
        CollectReplayError::UnknownEventType { .. }
        | CollectReplayError::Access(_)
        | CollectReplayError::UnsupportedAuthPlugin { .. } => not_found(event_type),
        CollectReplayError::Backend(detail) => {
            warn!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_name = "mcp.tool.replay.failed",
                error = %detail,
                "MCP replay failed in the notification backend"
            );
            REPLAY_FAILED.to_string()
        }
        other => other.to_string(),
    }
}

fn default_from_date() -> String {
    (Utc::now() - ChronoDuration::hours(DEFAULT_REPLAY_WINDOW_HOURS))
        .to_rfc3339_opts(SecondsFormat::Secs, true)
}

#[cfg(test)]
mod tests {
    use super::{REPLAY_FAILED, no_arguments, replay_error};
    use crate::sse::collect::CollectReplayError;
    use serde_json::json;

    #[test]
    fn backend_failures_do_not_reach_the_model() {
        let error = CollectReplayError::Backend("nats://10.0.0.7:4222 refused".to_string());
        assert_eq!(replay_error("open", error), REPLAY_FAILED);
    }

    #[test]
    fn argument_free_tools_reject_arguments() {
        assert!(no_arguments(json!({})).is_ok());
        assert!(no_arguments(json!({"x": 1})).is_err());
    }
}
