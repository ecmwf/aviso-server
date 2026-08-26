// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use crate::helpers::spawn_streaming_test_app;
use crate::test_utils::unique_suffix;
use reqwest::StatusCode;
use serde_json::{Value, json};
use tokio::time::{Duration, Instant, sleep, timeout};

async fn publish_cloud(
    client: &reqwest::Client,
    address: &str,
    marker: &str,
    cloud: Value,
) -> reqwest::Response {
    publish_cloud_for_event(client, address, "test_point_cloud", marker, cloud).await
}

async fn publish_cloud_for_event(
    client: &reqwest::Client,
    address: &str,
    event_type: &str,
    marker: &str,
    cloud: Value,
) -> reqwest::Response {
    client
        .post(format!("{address}/api/v1/notification"))
        .json(&json!({
            "event_type": event_type,
            "identifier": {
                "date": "20260826",
                "point_cloud": cloud
            },
            "payload": {"marker": marker}
        }))
        .send()
        .await
        .expect("point-cloud notification request")
}

fn polygon_filter() -> Value {
    json!([[0, 0], [0, 10], [10, 10], [10, 0], [0, 0]])
}

fn cloud_event_matching(body: &str, marker: &str) -> Option<Value> {
    body.lines()
        .filter_map(|line| line.strip_prefix("data:").map(str::trim))
        .filter(|line| line.contains(marker))
        .filter_map(|line| serde_json::from_str(line).ok())
        .find(|event: &Value| {
            event.get("type").and_then(Value::as_str) == Some("int.ecmwf.aviso.test_point_cloud")
        })
}

#[tokio::test]
async fn point_cloud_replay_filters_and_round_trips_array_without_subject_coordinates() {
    let app = spawn_streaming_test_app().await;
    let client = reqwest::Client::new();
    let marker = format!("POINT_CLOUD_REPLAY_{}", unique_suffix());
    let cloud = json!([[20.0, 20.0], [0.0, 5.0], [0.0, 5.0]]);
    let response = publish_cloud(&client, &app.address, &marker, cloud.clone()).await;
    assert_eq!(response.status(), StatusCode::OK);

    let replay = client
        .post(format!("{}/api/v1/replay", app.address))
        .json(&json!({
            "event_type": "test_point_cloud",
            "identifier": {
                "date": "20260826",
                "polygon": polygon_filter()
            },
            "from_id": "0"
        }))
        .send()
        .await
        .expect("point-cloud replay request");
    assert_eq!(replay.status(), StatusCode::OK);
    let body = replay.text().await.expect("replay body");
    let event = cloud_event_matching(&body, &marker).expect("matching replay CloudEvent");
    assert_eq!(event["data"]["identifier"]["point_cloud"], cloud);
    assert_eq!(event["data"]["identifier"]["date"], json!("20260826"));
}

#[tokio::test]
async fn point_cloud_live_watch_matches_boundary_and_excludes_outside_cloud() {
    let app = spawn_streaming_test_app().await;
    let client = reqwest::Client::new();
    let included = format!("POINT_CLOUD_LIVE_IN_{}", unique_suffix());
    let excluded = format!("POINT_CLOUD_LIVE_OUT_{}", unique_suffix());
    let mut watch = client
        .post(format!("{}/api/v1/watch", app.address))
        .json(&json!({
            "event_type": "test_point_cloud",
            "identifier": {
                "date": "20260826",
                "polygon": polygon_filter()
            }
        }))
        .send()
        .await
        .expect("point-cloud watch request");
    assert_eq!(watch.status(), StatusCode::OK);

    sleep(Duration::from_millis(100)).await;
    assert_eq!(
        publish_cloud(
            &client,
            &app.address,
            &excluded,
            json!([[20.0, 20.0], [30.0, 30.0]])
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert_eq!(
        publish_cloud(&client, &app.address, &included, json!([[0.0, 0.0]]))
            .await
            .status(),
        StatusCode::OK
    );

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut observed = String::new();
    while Instant::now() < deadline && !observed.contains(&included) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match timeout(remaining, watch.chunk()).await {
            Ok(Ok(Some(chunk))) => observed.push_str(&String::from_utf8_lossy(&chunk)),
            Ok(Ok(None)) | Err(_) => break,
            Ok(Err(error)) => panic!("watch read failed: {error}"),
        }
    }
    assert!(observed.contains(&included), "boundary cloud must match");
    assert!(
        !observed.contains(&excluded),
        "outside cloud must not match"
    );
    let event = cloud_event_matching(&observed, &included).expect("live point-cloud CloudEvent");
    assert_eq!(
        event["data"]["identifier"]["point_cloud"],
        json!([[0.0, 0.0]])
    );
}

#[tokio::test]
async fn point_cloud_rejects_string_empty_malformed_and_excess_points() {
    let app = spawn_streaming_test_app().await;
    let client = reqwest::Client::new();
    let cases = [
        json!("1,2,3,4"),
        json!([]),
        json!([[1.0]]),
        json!([[91.0, 0.0]]),
    ];
    for cloud in cases {
        let response = publish_cloud(&client, &app.address, "INVALID", cloud).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    let too_many = Value::Array(vec![json!([1.0, 2.0]); 10_001]);
    let response = publish_cloud(&client, &app.address, "TOO_MANY", too_many).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body: Value = response.json().await.expect("max-points error body");
    assert_eq!(
        body["message"],
        "field 'point_cloud' point cloud contains 10001 points, maximum is 10000"
    );

    let oversized = Value::Array(vec![
        json!([-89.12345678901234, -179.12345678901235]);
        5_000
    ]);
    let serialized_size = oversized.to_string().len();
    let response = publish_cloud(&client, &app.address, "OVERSIZED", oversized).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body: Value = response.json().await.expect("serialized-size error body");
    assert_eq!(
        body["message"],
        format!(
            "field 'point_cloud' point cloud canonical JSON is {serialized_size} bytes, maximum is {} bytes",
            aviso_validators::MAX_SERIALIZED_POINT_CLOUD_BYTES
        )
    );
}

#[tokio::test]
async fn custom_max_points_accepts_exact_limit_and_reports_overflow() {
    let app = spawn_streaming_test_app().await;
    let client = reqwest::Client::new();
    let accepted = publish_cloud_for_event(
        &client,
        &app.address,
        "test_point_cloud_small",
        "EXACT_CUSTOM_MAX",
        json!([[1.0, 2.0], [3.0, 4.0]]),
    )
    .await;
    assert_eq!(accepted.status(), StatusCode::OK);

    let rejected = publish_cloud_for_event(
        &client,
        &app.address,
        "test_point_cloud_small",
        "OVER_CUSTOM_MAX",
        json!([[1.0, 2.0], [3.0, 4.0], [5.0, 6.0]]),
    )
    .await;
    assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
    let body: Value = rejected.json().await.expect("custom max error body");
    assert_eq!(
        body["message"],
        "field 'point_cloud' point cloud contains 3 points, maximum is 2"
    );
}

#[tokio::test]
async fn point_cloud_is_rejected_as_subscriber_filter() {
    let app = spawn_streaming_test_app().await;
    let client = reqwest::Client::new();
    for identifier in [
        json!({"date":"20260826","point_cloud":[[1.0,2.0]]}),
        json!({"date":"20260826","point":[1.0,2.0]}),
        json!({"date":"20260826"}),
    ] {
        let response = client
            .post(format!("{}/api/v1/watch", app.address))
            .json(&json!({
                "event_type": "test_point_cloud",
                "identifier": identifier
            }))
            .send()
            .await
            .expect("invalid watch request");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
