// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use crate::helpers::spawn_streaming_test_app;
use serde_json::{Value, json};

const UNKNOWN_EVENT: &str = "asadasdasd-this-is-not-in-the-schema";

/// Event types `set_streaming_test_notification_schema` configures and that
/// MUST appear in the strict-mode rejection error body. Asserting their
/// presence catches the regression where the schema silently drops an expected
/// event type (the most important case Copilot flagged).
///
/// We deliberately do NOT assert exact equality here: Rust integration tests
/// share a single `OnceLock`-stored global `notification_schema`, so the actual
/// list at runtime is the UNION of every test scaffolding that races to
/// initialize the global. That cross-pollution would make an exact-set
/// assertion flaky across test execution orderings.
const REQUIRED_CONFIGURED_EVENT_TYPES: &[&str] = &[
    "dissemination",
    "extreme",
    "mars",
    "test_polygon",
    "test_polygon_js",
    "test_polygon_optional",
];

#[tokio::test]
async fn post_notification_rejects_unknown_event_type_with_400_in_strict_mode() {
    let app = spawn_streaming_test_app().await;
    let response = reqwest::Client::new()
        .post(format!("{}/api/v1/notification", app.address))
        .json(&json!({
            "event_type": UNKNOWN_EVENT,
            "identifier": {"class": "od", "date": "20260521"},
            "payload": {"hello": "world"},
        }))
        .send()
        .await
        .expect("failed to call notify endpoint");

    assert_eq!(
        response.status().as_u16(),
        400,
        "strict mode must reject unknown event_type on /notification"
    );
    let body: Value = response
        .json()
        .await
        .expect("error body must be valid JSON");
    assert_eq!(
        body.get("code").and_then(Value::as_str),
        Some("UNKNOWN_EVENT_TYPE")
    );
    let configured = body
        .get("configured_event_types")
        .and_then(Value::as_array)
        .expect("error body must include configured_event_types list");
    let names: Vec<&str> = configured.iter().filter_map(Value::as_str).collect();

    for required in REQUIRED_CONFIGURED_EVENT_TYPES {
        assert!(
            names.contains(required),
            "configured_event_types {names:?} must include {required:?} \
             (regression: an expected event type went missing from the schema)"
        );
    }

    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(
        names, sorted,
        "configured_event_types must be sorted for deterministic client tooling"
    );
}

#[tokio::test]
async fn post_watch_rejects_unknown_event_type_with_400_in_strict_mode() {
    let app = spawn_streaming_test_app().await;
    let response = reqwest::Client::new()
        .post(format!("{}/api/v1/watch", app.address))
        .json(&json!({
            "event_type": UNKNOWN_EVENT,
            "identifier": {},
        }))
        .send()
        .await
        .expect("failed to call watch endpoint");

    assert_eq!(
        response.status().as_u16(),
        400,
        "strict mode must reject unknown event_type on /watch"
    );
    let body: Value = response
        .json()
        .await
        .expect("error body must be valid JSON");
    assert_eq!(
        body.get("code").and_then(Value::as_str),
        Some("UNKNOWN_EVENT_TYPE")
    );
}

#[tokio::test]
async fn post_replay_rejects_unknown_event_type_with_400_in_strict_mode() {
    let app = spawn_streaming_test_app().await;
    let response = reqwest::Client::new()
        .post(format!("{}/api/v1/replay", app.address))
        .json(&json!({
            "event_type": UNKNOWN_EVENT,
            "identifier": {},
            "from_date": "2025-01-01T00:00:00Z",
        }))
        .send()
        .await
        .expect("failed to call replay endpoint");

    assert_eq!(
        response.status().as_u16(),
        400,
        "strict mode must reject unknown event_type on /replay"
    );
    let body: Value = response
        .json()
        .await
        .expect("error body must be valid JSON");
    assert_eq!(
        body.get("code").and_then(Value::as_str),
        Some("UNKNOWN_EVENT_TYPE")
    );
}
