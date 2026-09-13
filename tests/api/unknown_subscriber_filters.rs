// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use crate::helpers::{
    spawn_jetstream_test_app, spawn_streaming_test_app, spawn_streaming_test_app_with_auth,
    test_jwt,
};
use reqwest::{Client, StatusCode};
use serde_json::{Value, json};

#[tokio::test]
async fn in_memory_unknown_subscriber_filters() {
    let app = spawn_streaming_test_app().await;
    check_filters(&app.address).await;
}

#[tokio::test]
async fn jetstream_unknown_subscriber_filters() {
    if !std::env::var("AVISO_RUN_NATS_TESTS")
        .is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
    {
        return;
    }
    let app = spawn_jetstream_test_app().await;
    check_filters(&app.address).await;
}

async fn check_filters(address: &str) {
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();
    let marker = format!("UNKNOWN_FILTER_{}", uuid::Uuid::new_v4());
    let published = client
        .post(format!("{address}/api/v1/notification"))
        .json(&json!({
            "event_type": "extreme",
            "identifier": {"region": "north", "run_time": "1200", "severity": 4, "anomaly": 10},
            "payload": {"marker": marker}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(published.status(), StatusCode::OK);
    for endpoint in ["watch", "replay"] {
        for value in [json!(4), json!({"gte": 4})] {
            let response = client
                .post(format!("{address}/api/v1/{endpoint}"))
                .json(&json!({
                    "event_type": "extreme",
                    "identifier": {"severtiy": value},
                    "from_id": "0"
                }))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            assert_eq!(response.headers()["content-type"], "application/json");
            let body: Value = response.json().await.unwrap();
            assert!(!body.to_string().contains(&marker));
            assert!(body["message"].as_str().unwrap().contains(&format!(
                "Unknown field 'severtiy' provided for {endpoint} operation"
            )));
        }

        let polygon = json!([[0, 0], [0, 1], [1, 1], [0, 0]]);
        for (event_type, identifier, expected_error) in [
            ("extreme", json!({"severity": {"gte": 4}}), None),
            ("test_polygon", json!({"polygon": polygon}), None),
            ("test_polygon", json!({"point": [0, 0]}), None),
            (
                "test_point_cloud",
                json!({"date": "20260826", "polygon": polygon}),
                None,
            ),
            ("test_polygon", json!({}), Some("Required field 'polygon'")),
            (
                "test_polygon",
                json!({"polgyon": polygon}),
                Some("Unknown field 'polgyon'"),
            ),
            (
                "test_point_cloud",
                json!({"date": "20260826"}),
                Some("Required field 'point_cloud'"),
            ),
            (
                "test_point_cloud",
                json!({"date": "20260826", "point_cloud": [[0, 0]]}),
                Some("identifier.point_cloud is only supported for notify"),
            ),
            (
                "test_point_cloud",
                json!({"date": "20260826", "point": [0, 0]}),
                Some("identifier.point is not supported for point-cloud schemas"),
            ),
        ] {
            let response = client
                .post(format!("{address}/api/v1/{endpoint}"))
                .json(&json!({
                    "event_type": event_type,
                    "identifier": identifier,
                    "from_id": "0"
                }))
                .send()
                .await
                .unwrap();
            if let Some(error) = expected_error {
                assert_eq!(response.status(), StatusCode::BAD_REQUEST);
                let body = response.text().await.unwrap();
                assert!(body.contains(error), "{body}");
            } else {
                assert_eq!(
                    response.status(),
                    StatusCode::OK,
                    "{event_type}: {identifier}"
                );
            }
        }
    }
}

#[tokio::test]
async fn unknown_subscriber_filters_do_not_precede_stream_auth() {
    let app = spawn_streaming_test_app_with_auth().await;
    let client = Client::new();
    for endpoint in ["watch", "replay"] {
        for (token, status) in [
            (None, StatusCode::UNAUTHORIZED),
            (Some(test_jwt("reader", &["reader"])), StatusCode::FORBIDDEN),
            (Some(test_jwt("admin", &["admin"])), StatusCode::BAD_REQUEST),
        ] {
            let mut request = client
                .post(format!("{}/api/v1/{endpoint}", app.address))
                .json(&json!({
                    "event_type": "test_polygon_auth_admin",
                    "identifier": {"polgyon": "typo"},
                    "from_id": "0"
                }));
            if let Some(token) = token {
                request = request.bearer_auth(token);
            }
            let response = request.send().await.unwrap();
            assert_eq!(response.status(), status);
            let body = response.text().await.unwrap();
            assert_eq!(
                body.contains("Unknown field 'polgyon'"),
                status == StatusCode::BAD_REQUEST
            );
        }
    }
}
