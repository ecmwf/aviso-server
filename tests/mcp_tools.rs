// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! MCP tools over the in-memory backend with a seeded history.

use aviso_server::auth::User;
use aviso_server::configuration::{McpSettings, Settings};
use aviso_server::mcp::tools::{ToolCallError, ToolContext, ToolOutput, call, list};
use aviso_server::notification_backend::{NotificationBackend, build_backend};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

fn init_config() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        let identifier = json!({
            "class": {"type": "StringHandler", "required": true},
            "number": {"type": "IntHandler", "required": true}
        });
        let stream = |base: &str, auth: Value| {
            let mut schema = json!({
                "topic": {"base": base, "key_order": ["class", "number"]},
                "identifier": identifier,
                "payload": {"required": false}
            });
            if !auth.is_null() {
                schema["auth"] = auth;
            }
            schema
        };
        let mut config: Settings = serde_json::from_value(json!({
            "application": {"host": "127.0.0.1", "port": 0},
            "notification_backend": {"kind": "in_memory"},
            "notification_schema": {
                "open": stream("McpOpen", Value::Null),
                "guarded": stream("McpGuarded",
                    json!({"required": true, "read_roles": {"mcp": ["aviso-mcp"]}})),
                "denied": stream("McpDenied",
                    json!({"required": true, "read_roles": {"ecmwf": ["reader"]}})),
                "plugged": stream("McpPlugged",
                    json!({"required": true, "plugins": ["ecpds"]}))
            }
        }))
        .unwrap();
        config.watch_endpoint.replay_batch_delay_ms = 0;
        config.init_global_config();
    });
}

fn user(roles: &[&str]) -> User {
    User {
        username: "f00dbabe".to_string(),
        realm: Some("mcp".to_string()),
        roles: roles.iter().map(|role| role.to_string()).collect(),
        attributes: HashMap::new(),
    }
}

fn settings() -> McpSettings {
    McpSettings {
        max_results: 5,
        ..McpSettings::default()
    }
}

async fn seeded_backend() -> Arc<dyn NotificationBackend> {
    init_config();
    let backend = build_backend(&serde_json::from_value(json!({"kind": "in_memory"})).unwrap())
        .await
        .unwrap();
    for number in 1..=8u64 {
        let class = if number % 2 == 0 { "od" } else { "rd" };
        for base in ["McpOpen", "McpGuarded", "McpDenied"] {
            backend
                .put_messages(
                    &format!("{base}.{class}.{number}"),
                    json!({"number": number}).to_string(),
                )
                .await
                .unwrap();
        }
    }
    backend
}

async fn run(
    backend: &Arc<dyn NotificationBackend>,
    roles: &[&str],
    name: &str,
    arguments: Value,
) -> Result<ToolOutput, ToolCallError> {
    let caller = user(roles);
    let settings = settings();
    let context = ToolContext {
        user: &caller,
        settings: &settings,
        backend: backend.clone(),
    };
    call(
        &context,
        Some(&json!({"name": name, "arguments": arguments})),
    )
    .await
}

async fn ok(backend: &Arc<dyn NotificationBackend>, name: &str, arguments: Value) -> Value {
    let output = run(backend, &["aviso-mcp"], name, arguments).await.unwrap();
    assert!(!output.is_error, "{}", output.result);
    let structured = output.result["structuredContent"].clone();
    let text = output.result["content"][0]["text"].as_str().unwrap();
    assert_eq!(serde_json::from_str::<Value>(text).unwrap(), structured);
    structured
}

async fn tool_error(
    backend: &Arc<dyn NotificationBackend>,
    roles: &[&str],
    name: &str,
    arguments: Value,
) -> String {
    let output = run(backend, roles, name, arguments).await.unwrap();
    assert!(output.is_error, "{}", output.result);
    assert_eq!(output.result["isError"], true);
    output.result["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string()
}

fn numbers(result: &Value) -> Vec<u64> {
    result["notifications"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["data"]["payload"]["number"].as_u64().unwrap())
        .collect()
}

#[test]
fn list_names_all_tools() {
    let names: Vec<String> = list()["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        names,
        [
            "aviso_health",
            "aviso_list_event_types",
            "aviso_describe_event_type",
            "aviso_replay_notifications",
            "aviso_check_data_available"
        ]
    );
}

#[tokio::test]
async fn health_reports_backend_state() {
    let backend = seeded_backend().await;
    let result = ok(&backend, "aviso_health", json!({})).await;
    assert_eq!(result["status"], "ok");
    assert_eq!(result["backend_connected"], true);
}

#[tokio::test]
async fn event_types_are_filtered_by_read_access() {
    let backend = seeded_backend().await;
    let result = ok(&backend, "aviso_list_event_types", json!({})).await;
    assert_eq!(result["event_types"], json!(["guarded", "open"]));
    let output = run(&backend, &[], "aviso_list_event_types", json!({}))
        .await
        .unwrap();
    assert_eq!(
        output.result["structuredContent"]["event_types"],
        json!(["open"])
    );
}

#[tokio::test]
async fn describe_matches_the_rest_schema_document() {
    let backend = seeded_backend().await;
    let result = ok(
        &backend,
        "aviso_describe_event_type",
        json!({"event_type": "open"}),
    )
    .await;
    let schema = aviso_server::routes::schema::lookup_event_schema("open").unwrap();
    let expected = aviso_server::routes::schema::event_schema_body("open", &schema);
    assert_eq!(result, expected);
    let message = tool_error(
        &backend,
        &["aviso-mcp"],
        "aviso_describe_event_type",
        json!({"event_type": "nope"}),
    )
    .await;
    assert_eq!(message, "Event type 'nope' not found");
}

#[tokio::test]
async fn describe_hides_event_types_the_caller_cannot_read() {
    let backend = seeded_backend().await;
    for (roles, event_type) in [
        (&[][..], "guarded"),
        (&["aviso-mcp"][..], "denied"),
        (&["aviso-mcp"][..], "plugged"),
    ] {
        let message = tool_error(
            &backend,
            roles,
            "aviso_describe_event_type",
            json!({"event_type": event_type}),
        )
        .await;
        assert_eq!(message, format!("Event type '{event_type}' not found"));
    }
}

#[tokio::test]
async fn replay_defaults_to_recent_history_and_caps_results() {
    let backend = seeded_backend().await;
    let arguments = json!({
        "event_type": "open",
        "identifier": {"class": "od", "number": {"gte": 0}}
    });
    let result = ok(&backend, "aviso_replay_notifications", arguments).await;
    assert_eq!(numbers(&result), vec![2, 4, 6, 8]);
    assert_eq!(result["truncated"], false);
    assert_eq!(result["count"], 4);

    let capped = json!({
        "event_type": "open",
        "identifier": {"class": "*", "number": {"gte": 0}},
        "from_id": "1",
        "max_results": 50
    });
    let result = ok(&backend, "aviso_replay_notifications", capped).await;
    assert_eq!(numbers(&result), vec![1, 2, 3, 4, 5], "server cap is 5");
    assert_eq!(result["truncated"], true);
}

#[tokio::test]
async fn replay_of_old_window_is_empty() {
    let backend = seeded_backend().await;
    let arguments = json!({
        "event_type": "open",
        "identifier": {"class": "od", "number": {"gte": 0}},
        "from_date": "2001-01-01T00:00:00Z",
        "to_date": "2001-01-02T00:00:00Z"
    });
    let result = ok(&backend, "aviso_replay_notifications", arguments).await;
    assert_eq!(result["count"], 0);
}

#[tokio::test]
async fn replay_respects_stream_read_roles() {
    let backend = seeded_backend().await;
    let arguments = |event_type: &str| {
        json!({
            "event_type": event_type,
            "identifier": {"class": "od", "number": {"gte": 0}},
            "from_id": "1"
        })
    };
    let result = ok(&backend, "aviso_replay_notifications", arguments("guarded")).await;
    assert_eq!(numbers(&result), vec![2, 4, 6, 8]);
    for (roles, event_type) in [(&[][..], "guarded"), (&["aviso-mcp"][..], "denied")] {
        let message = tool_error(
            &backend,
            roles,
            "aviso_replay_notifications",
            arguments(event_type),
        )
        .await;
        assert!(message.contains("read role"), "{message}");
    }
    let message = tool_error(
        &backend,
        &["aviso-mcp"],
        "aviso_replay_notifications",
        arguments("plugged"),
    )
    .await;
    assert!(message.contains("plugin"), "{message}");
}

#[tokio::test]
async fn bad_arguments_are_tool_errors() {
    let backend = seeded_backend().await;
    let cases = [
        ("aviso_replay_notifications", json!({"event_type": "open"})),
        (
            "aviso_replay_notifications",
            json!({"event_type": "open", "identifier": {}, "extra": 1}),
        ),
        (
            "aviso_replay_notifications",
            json!({"event_type": "missing", "identifier": {"class": "od"}}),
        ),
        (
            "aviso_replay_notifications",
            json!({"event_type": "open", "identifier": {"class": "od", "number": 1},
                   "max_results": 0}),
        ),
        ("aviso_health", json!({"verbose": true})),
    ];
    for (name, arguments) in cases {
        tool_error(&backend, &["aviso-mcp"], name, arguments).await;
    }
}

#[tokio::test]
async fn unknown_tool_and_malformed_call_are_protocol_errors() {
    let backend = seeded_backend().await;
    let result = run(&backend, &[], "aviso_write", json!({})).await;
    assert_eq!(
        result.unwrap_err(),
        ToolCallError::UnknownTool("aviso_write".to_string())
    );
    let caller = user(&[]);
    let settings = settings();
    let context = ToolContext {
        user: &caller,
        settings: &settings,
        backend,
    };
    assert_eq!(
        call(&context, Some(&json!({"arguments": {}})))
            .await
            .unwrap_err(),
        ToolCallError::MalformedCall
    );
}

#[tokio::test]
async fn data_availability_returns_the_oldest_match() {
    let backend = seeded_backend().await;
    let found = ok(
        &backend,
        "aviso_check_data_available",
        json!({"event_type": "open", "identifier": {"class": "od", "number": 6}}),
    )
    .await;
    assert_eq!(found["available"], true);
    assert_eq!(found["notification"]["data"]["payload"]["number"], 6);
    assert_eq!(found["first_seen"], found["notification"]["time"]);

    let missing = ok(
        &backend,
        "aviso_check_data_available",
        json!({"event_type": "open", "identifier": {"class": "od", "number": 7}}),
    )
    .await;
    assert_eq!(
        missing,
        json!({"available": false, "first_seen": null, "notification": null})
    );
}
