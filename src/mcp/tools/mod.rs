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
use crate::auth::stream_access::{StreamOperation, check_stream_access};
use crate::configuration::{McpSettings, Settings};
use crate::notification_backend::NotificationBackend;
use crate::telemetry::SERVICE_VERSION;
use definitions::{HEALTH, LIST_EVENT_TYPES};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;

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

/// MCP users are never admins, so access checks use an empty admin map.
fn no_admin_roles() -> HashMap<String, Vec<String>> {
    HashMap::new()
}

fn health(context: &ToolContext<'_>) -> Value {
    let connected = context.backend.connection_healthy();
    json!({
        "status": if connected { "ok" } else { "degraded" },
        "backend_connected": connected,
        "server_version": SERVICE_VERSION
    })
}

/// Configured event types the caller may read. Streams that need an auth
/// plugin are left out because MCP callers cannot pass plugin checks.
fn list_event_types(context: &ToolContext<'_>) -> Value {
    let admin_roles = no_admin_roles();
    let mut event_types: Vec<&String> = Settings::get_global_notification_schema()
        .as_ref()
        .map(|schemas| {
            schemas
                .iter()
                .filter(|(_, schema)| {
                    schema
                        .auth
                        .as_ref()
                        .and_then(|auth| auth.plugins.as_ref())
                        .is_none_or(Vec::is_empty)
                })
                .map(|(name, _)| name)
                .filter(|name| {
                    check_stream_access(
                        true,
                        Some(context.user),
                        Some(&admin_roles),
                        name,
                        StreamOperation::Read,
                    )
                    .is_ok()
                })
                .collect()
        })
        .unwrap_or_default();
    event_types.sort();
    json!({"event_types": event_types})
}

#[cfg(test)]
mod tests {
    use super::no_arguments;
    use serde_json::json;

    #[test]
    fn argument_free_tools_reject_arguments() {
        assert!(no_arguments(json!({})).is_ok());
        assert!(no_arguments(json!({"x": 1})).is_err());
    }
}
