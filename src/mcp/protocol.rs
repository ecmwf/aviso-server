// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! JSON-RPC 2.0 message handling and MCP version negotiation.

use serde_json::{Map, Value, json};

/// Every revision accepted in the `MCP-Protocol-Version` header, newest
/// first.
pub const SUPPORTED_PROTOCOL_VERSIONS: [&str; 3] = ["2025-11-25", "2025-06-18", "2025-03-26"];
/// Newest protocol revision this server speaks.
pub const LATEST_PROTOCOL_VERSION: &str = SUPPORTED_PROTOCOL_VERSIONS[0];
/// Revision assumed when a request has no `MCP-Protocol-Version` header.
pub const DEFAULT_PROTOCOL_VERSION: &str = "2025-03-26";

/// JSON-RPC error codes used by this server.
pub mod codes {
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
}

/// A JSON-RPC error object.
#[derive(Debug, Clone, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

impl RpcError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(codes::INVALID_PARAMS, message)
    }
}

/// One incoming JSON-RPC message, classified.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// Has `method` and `id`; expects a response.
    Request {
        id: Value,
        method: String,
        params: Option<Value>,
    },
    /// Has `method` and no `id`; gets no response.
    Notification { method: String },
    /// A response to a server request. This server sends none, so these
    /// are acknowledged and dropped.
    Response,
}

/// Parse one JSON-RPC message from a request body.
///
/// - valid: `{"jsonrpc":"2.0","id":1,"method":"ping"}`
/// - invalid: `[{"jsonrpc":"2.0","id":1,"method":"ping"}]` (batch),
///   `{"jsonrpc":"1.0","id":1,"method":"ping"}`
pub fn parse_message(body: &[u8]) -> Result<Message, RpcError> {
    let value: Value = serde_json::from_slice(body)
        .map_err(|_| RpcError::new(codes::PARSE_ERROR, "Parse error"))?;
    let invalid = |message: &str| RpcError::new(codes::INVALID_REQUEST, message);
    let Value::Object(object) = value else {
        return Err(invalid("Expected exactly one JSON-RPC message object"));
    };
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(invalid("jsonrpc must be \"2.0\""));
    }
    let id = object.get("id");
    if id.is_some_and(|id| !(id.is_string() || id.is_number())) {
        return Err(invalid("id must be a string or a number"));
    }
    let params = object.get("params");
    if params.is_some_and(|params| !(params.is_object() || params.is_array())) {
        return Err(invalid("params must be an object or an array"));
    }
    match (object.get("method"), id) {
        (Some(Value::String(method)), Some(id)) => Ok(Message::Request {
            id: id.clone(),
            method: method.clone(),
            params: params.cloned(),
        }),
        (Some(Value::String(method)), None) => Ok(Message::Notification {
            method: method.clone(),
        }),
        (Some(_), _) => Err(invalid("method must be a string")),
        (None, Some(_)) if is_response(&object) => Ok(Message::Response),
        (None, _) => Err(invalid("Missing method")),
    }
}

fn is_response(object: &Map<String, Value>) -> bool {
    object.contains_key("result") ^ object.contains_key("error")
}

/// A success response.
pub fn success(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

/// An error response. `id` is `null` when the request id is unknown.
pub fn failure(id: Option<&Value>, error: &RpcError) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id.cloned().unwrap_or(Value::Null),
        "error": {"code": error.code, "message": error.message}
    })
}

/// Whether `version` is accepted in the `MCP-Protocol-Version` header.
pub fn is_supported_version(version: &str) -> bool {
    SUPPORTED_PROTOCOL_VERSIONS.contains(&version)
}

/// Pick the revision to answer `initialize` with: the client's when
/// supported, otherwise the newest.
pub fn negotiate_version(requested: Option<&str>) -> &'static str {
    requested
        .and_then(|requested| {
            SUPPORTED_PROTOCOL_VERSIONS
                .iter()
                .copied()
                .find(|supported| *supported == requested)
        })
        .unwrap_or(LATEST_PROTOCOL_VERSION)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(value: Value) -> Result<Message, RpcError> {
        parse_message(value.to_string().as_bytes())
    }

    #[test]
    fn classifies_requests_notifications_and_responses() {
        let request = parse(json!({"jsonrpc": "2.0", "id": "a", "method": "ping"}));
        assert!(matches!(request, Ok(Message::Request { .. })));
        let note = parse(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        assert!(matches!(note, Ok(Message::Notification { .. })));
        let response = parse(json!({"jsonrpc": "2.0", "id": 3, "result": {}}));
        assert_eq!(response, Ok(Message::Response));
    }

    #[test]
    fn rejects_batches_and_malformed_envelopes() {
        let code = |value: Value| parse(value).map(|_| ()).unwrap_err().code;
        assert_eq!(
            code(json!([{"jsonrpc": "2.0", "id": 1, "method": "ping"}])),
            codes::INVALID_REQUEST
        );
        assert_eq!(
            code(json!({"jsonrpc": "1.0", "id": 1, "method": "ping"})),
            codes::INVALID_REQUEST
        );
        assert_eq!(
            code(json!({"jsonrpc": "2.0", "id": null, "method": "ping"})),
            codes::INVALID_REQUEST
        );
        assert_eq!(
            code(json!({"jsonrpc": "2.0", "id": 1, "method": 5})),
            codes::INVALID_REQUEST
        );
        assert_eq!(
            code(json!({"jsonrpc": "2.0", "id": 1, "method": "x", "params": 1})),
            codes::INVALID_REQUEST
        );
        assert_eq!(
            parse_message(b"{not json").unwrap_err().code,
            codes::PARSE_ERROR
        );
    }

    #[test]
    fn negotiation_echoes_known_versions_only() {
        assert_eq!(negotiate_version(Some("2025-06-18")), "2025-06-18");
        assert_eq!(
            negotiate_version(Some("2024-11-05")),
            LATEST_PROTOCOL_VERSION
        );
        assert_eq!(negotiate_version(None), LATEST_PROTOCOL_VERSION);
    }
}
