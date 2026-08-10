// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use crate::helpers::spawn_app;

#[tokio::test]

async fn health_check_works() {
    let app = spawn_app().await;

    let client = reqwest::Client::new();

    let response = client
        .get(format!("{}/health", &app.address))
        .send()
        .await
        .expect("Failed to send request");

    assert!(response.status().is_success());
}

#[tokio::test]
async fn ready_reports_connected_backend() {
    // The test server runs the in-memory backend, which has no remote
    // connection and therefore always reports healthy; readiness must be
    // 200 with the backend marked connected. The disconnected 503 path is
    // covered by handler-level tests with a scripted backend.
    let app = spawn_app().await;

    let client = reqwest::Client::new();

    let response = client
        .get(format!("{}/ready", &app.address))
        .send()
        .await
        .expect("Failed to send request");

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = response.json().await.expect("valid json body");
    assert_eq!(body["status"], "ready");
    assert_eq!(body["backend_connected"], true);
}
