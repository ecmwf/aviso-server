// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use crate::helpers::{spawn_app, spawn_streaming_test_app};
use crate::test_utils::test_polygon;

const HEADER: &str = "x-request-id";

fn assert_uuid_v4(value: &str) {
    let re = regex::Regex::new(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")
        .expect("valid uuid regex");
    assert!(re.is_match(value), "expected UUID v4, got: {value}");
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
    assert_uuid_v4(header.to_str().expect("header should be ascii"));
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
    assert_uuid_v4(&header_id);

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
    assert_uuid_v4(header.to_str().expect("header should be ascii"));
}
