// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Route one MCP request to its method handler.

use crate::mcp::protocol::{RpcError, codes, negotiate_version};
use crate::mcp::tools::{self, ToolContext};
use crate::telemetry::SERVICE_VERSION;
use serde_json::{Value, json};

/// MCP method names this server knows. Requests use the first four;
/// the notifications are accepted and need no answer.
pub mod methods {
    pub const INITIALIZE: &str = "initialize";
    pub const PING: &str = "ping";
    pub const TOOLS_LIST: &str = "tools/list";
    pub const TOOLS_CALL: &str = "tools/call";
    pub const INITIALIZED: &str = "notifications/initialized";
    pub const CANCELLED: &str = "notifications/cancelled";

    /// Every known name, for logging client input safely.
    pub const ALL: [&str; 6] = [
        INITIALIZE,
        PING,
        TOOLS_LIST,
        TOOLS_CALL,
        INITIALIZED,
        CANCELLED,
    ];
}

/// Result of one method call.
pub struct Dispatched {
    /// The JSON-RPC `result`, or the error to answer with.
    pub result: Result<Value, RpcError>,
    /// Tool name for `tools/call`.
    pub tool: Option<String>,
    /// True when a tool ran and reported `isError: true`.
    pub tool_error: bool,
}

impl Dispatched {
    fn plain(result: Result<Value, RpcError>) -> Self {
        Self {
            result,
            tool: None,
            tool_error: false,
        }
    }
}

/// Run `method` with `params`.
pub async fn dispatch(
    context: &ToolContext<'_>,
    method: &str,
    params: Option<Value>,
) -> Dispatched {
    match method {
        methods::INITIALIZE => Dispatched::plain(initialize(params.as_ref())),
        methods::PING => Dispatched::plain(Ok(json!({}))),
        methods::TOOLS_LIST => Dispatched::plain(Ok(tools::list())),
        methods::TOOLS_CALL => match tools::call(context, params.as_ref()).await {
            Ok(output) => Dispatched {
                result: Ok(output.result),
                tool: Some(output.name),
                tool_error: output.is_error,
            },
            Err(error) => Dispatched::plain(Err(RpcError::invalid_params(error.to_string()))),
        },
        _ => Dispatched::plain(Err(RpcError::new(
            codes::METHOD_NOT_FOUND,
            format!("Method not found: {method}"),
        ))),
    }
}

fn initialize(params: Option<&Value>) -> Result<Value, RpcError> {
    let params = params
        .and_then(Value::as_object)
        .ok_or_else(|| RpcError::invalid_params("initialize requires params"))?;
    let requested = params.get("protocolVersion").and_then(Value::as_str);
    if requested.is_none() {
        return Err(RpcError::invalid_params("protocolVersion is required"));
    }
    Ok(json!({
        "protocolVersion": negotiate_version(requested),
        "capabilities": {"tools": {}},
        "serverInfo": {"name": "aviso-server", "version": SERVICE_VERSION},
        "instructions": "Read-only access to Aviso notifications. Use \
            aviso_list_event_types first, then aviso_describe_event_type to \
            learn the identifier fields of an event type."
    }))
}
