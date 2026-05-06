// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use crate::helpers::{spawn_app, spawn_streaming_test_app};
use crate::test_utils::test_polygon;
use regex::Regex;
use serde_json::Value;
use std::sync::LazyLock;

const HEADER: &str = "x-request-id";

// The exact UUID version (v4 today, v7 if tracing-actix-web's uuid_v7 feature
// is ever enabled upstream) is not part of aviso's contract; we only assert
// the canonical hyphenated lowercase shape. Compiled once for the whole test
// module to avoid recompiling on every assertion.
static UUID_FORMAT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")
        .expect("valid uuid regex")
});

fn assert_uuid_format(value: &str) {
    assert!(
        UUID_FORMAT_RE.is_match(value),
        "expected canonical UUID, got: {value}"
    );
}

fn extract_header_id(response: &reqwest::Response) -> String {
    response
        .headers()
        .get(HEADER)
        .expect("X-Request-ID header should be present")
        .to_str()
        .expect("header should be ascii")
        .to_owned()
}

#[tokio::test]
async fn x_request_id_header_present_on_health_check() {
    let app = spawn_app().await;

    let response = reqwest::Client::new()
        .get(format!("{}/health", &app.address))
        .send()
        .await
        .expect("request should succeed");

    assert!(response.status().is_success());
    let header = response
        .headers()
        .get(HEADER)
        .expect("X-Request-ID header should be present on /health");
    assert_uuid_format(header.to_str().expect("header should be ascii"));
}

#[tokio::test]
async fn x_request_id_header_present_on_4xx_validation_error() {
    // Trigger a 400 by posting an invalid notification body to /publish.
    let app = spawn_app().await;

    let response = reqwest::Client::new()
        .post(format!("{}/api/v1/notification", &app.address))
        .header("content-type", "application/json")
        .body("not-valid-json")
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(response.status().as_u16(), 400);
    let header_id = response
        .headers()
        .get(HEADER)
        .expect("X-Request-ID header should be present on 4xx error")
        .to_str()
        .expect("header should be ascii")
        .to_owned();
    assert_uuid_format(&header_id);

    let body: serde_json::Value = response
        .json()
        .await
        .expect("error body should be valid JSON");
    let body_id = body["request_id"]
        .as_str()
        .expect("error body should include request_id field");
    assert_eq!(
        body_id, header_id,
        "X-Request-ID header should match request_id in body"
    );
}

#[tokio::test]
async fn x_request_id_header_differs_across_requests() {
    let app = spawn_app().await;
    let client = reqwest::Client::new();

    let res_a = client
        .get(format!("{}/health", &app.address))
        .send()
        .await
        .expect("first request should succeed");
    let res_b = client
        .get(format!("{}/health", &app.address))
        .send()
        .await
        .expect("second request should succeed");

    let id_a = res_a
        .headers()
        .get(HEADER)
        .expect("first response should carry header")
        .to_str()
        .unwrap()
        .to_owned();
    let id_b = res_b
        .headers()
        .get(HEADER)
        .expect("second response should carry header")
        .to_str()
        .unwrap()
        .to_owned();

    assert_ne!(
        id_a, id_b,
        "every request should receive a fresh request id"
    );
}

#[tokio::test]
async fn x_request_id_header_present_on_sse_stream() {
    // The header is sent before any SSE bytes flow, so the stream's 200 OK
    // response carries it even though the body is open-ended.
    let app = spawn_streaming_test_app().await;
    let request_body = serde_json::json!({
        "event_type": "test_polygon",
        "identifier": {
            "time": "1200",
            "polygon": test_polygon(),
        }
    });

    let response = reqwest::Client::new()
        .post(format!("{}/api/v1/watch", &app.address))
        .header("content-type", "application/json")
        .json(&request_body)
        .send()
        .await
        .expect("watch request should succeed");

    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("text/event-stream"),
    );
    let header = response
        .headers()
        .get(HEADER)
        .expect("X-Request-ID header should be present on SSE 200 OK");
    assert_uuid_format(header.to_str().expect("header should be ascii"));
}

#[tokio::test]
async fn sse_connection_established_payload_includes_request_id() {
    // Stream the response just long enough to read the first SSE frame, parse
    // its data: payload as JSON, and confirm the connection_established event
    // exposes the same UUID as the X-Request-ID header. This is the actual
    // user-facing contract: a client that consumes the stream as raw bytes
    // (no header parsing) still sees the request id once, near the top.
    use std::time::Duration;

    let app = spawn_streaming_test_app().await;
    let request_body = serde_json::json!({
        "event_type": "test_polygon",
        "identifier": {
            "time": "1200",
            "polygon": test_polygon(),
        }
    });

    let response = reqwest::Client::new()
        .post(format!("{}/api/v1/watch", &app.address))
        .header("content-type", "application/json")
        .json(&request_body)
        .send()
        .await
        .expect("watch request should succeed");

    assert_eq!(response.status().as_u16(), 200);
    let header_id = response
        .headers()
        .get(HEADER)
        .expect("X-Request-ID header should be present")
        .to_str()
        .expect("header should be ascii")
        .to_owned();

    // Read chunks until we see the first complete event terminator (\n\n)
    // or hit a sane byte cap. Watch streams are open-ended, so we cannot
    // wait for body close.
    let mut response = response;
    let mut buffer = Vec::with_capacity(4096);
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Ok(Some(chunk)) = response.chunk().await {
            buffer.extend_from_slice(&chunk);
            if buffer.windows(2).any(|w| w == b"\n\n") || buffer.len() >= 4096 {
                break;
            }
        }
    })
    .await
    .expect("first SSE event should arrive within 5s");

    let text = String::from_utf8(buffer).expect("SSE bytes should be valid utf-8");
    let data_line = text
        .lines()
        .find(|line| line.starts_with("data: "))
        .expect("first event should contain a data: line");
    let payload: serde_json::Value =
        serde_json::from_str(&data_line[6..]).expect("data: line should hold valid JSON");

    assert_eq!(
        payload["type"], "connection_established",
        "first event should be connection_established"
    );
    let body_id = payload["request_id"]
        .as_str()
        .expect("connection_established payload should carry request_id");
    assert_eq!(
        body_id, header_id,
        "in-stream request_id should match X-Request-ID header"
    );
}

#[tokio::test]
async fn schema_get_404_body_includes_request_id() {
    let app = spawn_app().await;

    let response = reqwest::Client::new()
        .get(format!(
            "{}/api/v1/schema/event_does_not_exist",
            &app.address
        ))
        .send()
        .await
        .expect("schema request should succeed");

    assert_eq!(response.status().as_u16(), 404);
    let header_id = extract_header_id(&response);
    let body: Value = response.json().await.expect("body should be JSON");
    assert_eq!(
        body["request_id"]
            .as_str()
            .expect("body should include request_id"),
        header_id,
        "schema 404 body request_id should match X-Request-ID header"
    );
}

#[tokio::test]
async fn admin_delete_400_body_includes_request_id() {
    // Admin error bodies must carry request_id because aviso can be deployed
    // with auth disabled, which makes /api/v1/admin/* publicly reachable.
    let app = spawn_streaming_test_app().await;

    let response = reqwest::Client::new()
        .delete(format!(
            "{}/api/v1/admin/notification/not-a-valid-id",
            &app.address
        ))
        .send()
        .await
        .expect("admin request should succeed");

    assert_eq!(response.status().as_u16(), 400);
    let header_id = extract_header_id(&response);
    let body: Value = response.json().await.expect("body should be JSON");
    assert_eq!(
        body["request_id"]
            .as_str()
            .expect("body should include request_id"),
        header_id,
        "admin 400 body request_id should match X-Request-ID header"
    );
}

#[tokio::test]
async fn admin_delete_404_body_includes_request_id() {
    let app = spawn_streaming_test_app().await;

    let response = reqwest::Client::new()
        .delete(format!(
            "{}/api/v1/admin/notification/test_polygon@999999999",
            &app.address
        ))
        .send()
        .await
        .expect("admin request should succeed");

    assert_eq!(response.status().as_u16(), 404);
    let header_id = extract_header_id(&response);
    let body: Value = response.json().await.expect("body should be JSON");
    assert_eq!(
        body["request_id"]
            .as_str()
            .expect("body should include request_id"),
        header_id,
        "admin 404 body request_id should match X-Request-ID header"
    );
}
