// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Tool descriptors returned by `tools/list`.
//!
//! Each tool declares an `inputSchema` for its arguments and an
//! `outputSchema` for its `structuredContent`. Clients may validate results
//! against the output schema, so every field a tool returns is listed here.

use serde_json::{Value, json};

pub const HEALTH: &str = "aviso_health";
pub const LIST_EVENT_TYPES: &str = "aviso_list_event_types";
pub const DESCRIBE_EVENT_TYPE: &str = "aviso_describe_event_type";
pub const REPLAY_NOTIFICATIONS: &str = "aviso_replay_notifications";
pub const CHECK_DATA_AVAILABLE: &str = "aviso_check_data_available";

const READ_ONLY: &str = "readOnlyHint";

fn no_arguments() -> Value {
    json!({"type": "object", "properties": {}, "additionalProperties": false})
}

fn event_type_property() -> Value {
    json!({
        "type": "string",
        "description": "Event type name, as returned by aviso_list_event_types."
    })
}

fn identifier_property() -> Value {
    json!({
        "type": "object",
        "description": "Identifier fields to match. Field names and value \
            rules come from aviso_describe_event_type. Omitted optional \
            fields match any value.",
        "additionalProperties": true
    })
}

fn date_property(description: &str) -> Value {
    json!({
        "type": "string",
        "description": format!(
            "{description} RFC 3339 (2025-06-09T13:15:00Z) or unix seconds."
        )
    })
}

fn notification_schema() -> Value {
    json!({
        "type": "object",
        "description": "A CloudEvent, as delivered by the replay API.",
        "additionalProperties": true
    })
}

/// `schema` or `null`. Nullable values use an `anyOf` branch instead of a
/// `type` array, because some clients map tool schemas onto a dialect with
/// a single `type` per schema.
///
/// Valid: `{"anyOf": [{"type": "string"}, {"type": "null"}]}`.
/// Avoided: `{"type": ["string", "null"]}`.
fn nullable(schema: Value) -> Value {
    json!({"anyOf": [schema, {"type": "null"}]})
}

/// Descriptors for every tool, in a stable order.
pub fn all() -> Vec<Value> {
    vec![
        json!({
            "name": HEALTH,
            "title": "Aviso health",
            "description": "Report whether the Aviso server can reach its \
                notification backend.",
            "inputSchema": no_arguments(),
            "outputSchema": {
                "type": "object",
                "properties": {
                    "status": {"type": "string", "enum": ["ok", "degraded"]},
                    "backend_connected": {"type": "boolean"},
                    "server_version": {"type": "string"}
                },
                "required": ["status", "backend_connected", "server_version"]
            },
            "annotations": {READ_ONLY: true, "openWorldHint": false}
        }),
        json!({
            "name": LIST_EVENT_TYPES,
            "title": "List event types",
            "description": "List the event types you are allowed to read.",
            "inputSchema": no_arguments(),
            "outputSchema": {
                "type": "object",
                "properties": {
                    "event_types": {"type": "array", "items": {"type": "string"}}
                },
                "required": ["event_types"]
            },
            "annotations": {READ_ONLY: true, "openWorldHint": false}
        }),
        json!({
            "name": DESCRIBE_EVENT_TYPE,
            "title": "Describe event type",
            "description": "Show the identifier fields and payload rules of \
                one event type.",
            "inputSchema": {
                "type": "object",
                "properties": {"event_type": event_type_property()},
                "required": ["event_type"],
                "additionalProperties": false
            },
            "outputSchema": {
                "type": "object",
                "properties": {
                    "status": {"type": "string"},
                    "event_type": {"type": "string"},
                    "schema": {"type": "object"}
                },
                "required": ["status", "event_type", "schema"]
            },
            "annotations": {READ_ONLY: true, "openWorldHint": false}
        }),
        json!({
            "name": REPLAY_NOTIFICATIONS,
            "title": "Replay notifications",
            "description": "Return past notifications that match an \
                identifier, oldest first. Without from_date or from_id the \
                last 24 hours are searched. Results are capped; truncated is \
                true when more matches exist past the cap.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "event_type": event_type_property(),
                    "identifier": identifier_property(),
                    "from_date": date_property("Inclusive start time."),
                    "from_id": {
                        "type": "string",
                        "description": "Inclusive start sequence number. \
                            Use instead of from_date."
                    },
                    "to_date": date_property("Inclusive end time."),
                    "to_id": {
                        "type": "string",
                        "description": "Inclusive end sequence number."
                    },
                    "max_results": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Most notifications to return. \
                            Capped by the server limit."
                    }
                },
                "required": ["event_type", "identifier"],
                "additionalProperties": false
            },
            "outputSchema": {
                "type": "object",
                "properties": {
                    "event_type": {"type": "string"},
                    "count": {"type": "integer"},
                    "truncated": {"type": "boolean"},
                    "notifications": {"type": "array", "items": notification_schema()}
                },
                "required": ["event_type", "count", "truncated", "notifications"]
            },
            "annotations": {READ_ONLY: true, "openWorldHint": false}
        }),
        json!({
            "name": CHECK_DATA_AVAILABLE,
            "title": "Check data availability",
            "description": "Check whether any notification matching an \
                identifier is still held by Aviso, and return the oldest \
                one. Searches the whole retained history unless from_date \
                is given.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "event_type": event_type_property(),
                    "identifier": identifier_property(),
                    "from_date": date_property("Only consider notifications from this time.")
                },
                "required": ["event_type", "identifier"],
                "additionalProperties": false
            },
            "outputSchema": {
                "type": "object",
                "properties": {
                    "available": {"type": "boolean"},
                    "first_seen": nullable(json!({"type": "string"})),
                    "notification": nullable(notification_schema())
                },
                "required": ["available", "first_seen", "notification"]
            },
            "annotations": {READ_ONLY: true, "openWorldHint": false}
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::{CHECK_DATA_AVAILABLE, all};
    use serde_json::{Value, json};

    /// Paths of every node under `value` whose `type` is an array.
    fn type_arrays(value: &Value, path: &str, found: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                if map.get("type").is_some_and(Value::is_array) {
                    found.push(path.to_string());
                }
                for (key, child) in map {
                    type_arrays(child, &format!("{path}.{key}"), found);
                }
            }
            Value::Array(items) => {
                for (index, child) in items.iter().enumerate() {
                    type_arrays(child, &format!("{path}[{index}]"), found);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn no_schema_uses_a_type_array() {
        let mut found = Vec::new();
        for tool in all() {
            let name = tool["name"].as_str().unwrap();
            for key in ["inputSchema", "outputSchema"] {
                type_arrays(&tool[key], &format!("{name}.{key}"), &mut found);
            }
        }
        assert!(found.is_empty(), "type arrays at {found:?}");
    }

    #[test]
    fn nullable_output_fields_use_any_of() {
        let tool = all()
            .into_iter()
            .find(|tool| tool["name"] == CHECK_DATA_AVAILABLE)
            .unwrap();
        let properties = &tool["outputSchema"]["properties"];
        assert_eq!(
            properties["first_seen"],
            json!({"anyOf": [{"type": "string"}, {"type": "null"}]})
        );
        assert_eq!(
            properties["notification"]["anyOf"][1],
            json!({"type": "null"})
        );
    }

    #[test]
    fn every_tool_has_object_input_and_output_schemas() {
        let tools = all();
        assert_eq!(tools.len(), 5);
        for tool in tools {
            assert!(tool["name"].as_str().unwrap().starts_with("aviso_"));
            assert_eq!(tool["inputSchema"]["type"], "object");
            assert_eq!(tool["outputSchema"]["type"], "object");
            assert_eq!(tool["annotations"]["readOnlyHint"], true);
        }
    }
}
