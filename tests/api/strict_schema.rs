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

/// The exact, sorted set of event types the integration-test global schema
/// produces. The harness initializes it once via the
/// `TEST_GLOBAL_CONFIG.get_or_init` gate in `tests/api/helpers.rs` by calling
/// `ensure_test_notification_schema(.., include_auth_schemas = true)`, then
/// (under `--features ecpds`) appending the ECPDS entries. The same superset
/// is produced regardless of which test runs first, so we can pin it exactly.
///
/// Pinning exact equality (rather than a superset check) catches BOTH
/// regression classes Copilot flagged: an expected entry silently goes
/// missing, OR an unrelated entry leaks in.
#[cfg(not(feature = "ecpds"))]
const EXPECTED_CONFIGURED_EVENT_TYPES: &[&str] = &[
    "dissemination",
    "extreme",
    "mars",
    "test_polygon",
    "test_polygon_auth_admin",
    "test_polygon_auth_any",
    "test_polygon_auth_optional",
    "test_polygon_auth_write",
    "test_polygon_js",
    "test_polygon_optional",
];

#[cfg(feature = "ecpds")]
const EXPECTED_CONFIGURED_EVENT_TYPES: &[&str] = &[
    "dissemination",
    "dissemination_ecpds",
    "dissemination_ecpds_writable",
    "extreme",
    "mars",
    "test_polygon",
    "test_polygon_auth_admin",
    "test_polygon_auth_any",
    "test_polygon_auth_optional",
    "test_polygon_auth_write",
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
    assert_eq!(
        names, EXPECTED_CONFIGURED_EVENT_TYPES,
        "configured_event_types must match exactly (and be sorted): both \
         missing-entry and leaked-entry regressions are caught by this check"
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
