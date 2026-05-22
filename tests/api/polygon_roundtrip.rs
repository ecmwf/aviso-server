// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use crate::helpers::spawn_streaming_test_app;
use crate::test_utils::{post_test_polygon_notification_with_polygon, unique_suffix};
use reqwest::StatusCode;
use serde_json::{Value, json};
use tokio::time::{Duration, Instant, sleep, timeout};

#[tokio::test]
async fn replay_emits_cloudevent_carrying_the_producer_polygon_in_data_identifier() {
    let app = spawn_streaming_test_app().await;
    let client = reqwest::Client::new();
    let suffix = unique_suffix();

    let note = format!("POLYGON_ROUNDTRIP_{suffix}");
    let producer_polygon = "(50.0,10.0,52.0,10.0,52.0,12.0,50.0,12.0,50.0,10.0)";
    let publish =
        post_test_polygon_notification_with_polygon(&client, &app.address, &note, producer_polygon)
            .await;
    assert_eq!(publish.status(), StatusCode::OK);

    let replay = client
        .post(format!("{}/api/v1/replay", &app.address))
        .header("Content-Type", "application/json")
        .json(&json!({
            "event_type": "test_polygon",
            "identifier": {
                "time": "1200",
                "polygon": "(49.0,9.0,53.0,9.0,53.0,13.0,49.0,13.0,49.0,9.0)",
            },
            "from_id": "0",
        }))
        .send()
        .await
        .expect("failed to call replay endpoint");
    assert_eq!(replay.status(), StatusCode::OK);
    let body = replay
        .text()
        .await
        .expect("failed to read replay response body");

    let cloud_event = extract_cloud_event_matching(&body, &note).unwrap_or_else(|| {
        panic!("expected replay body to contain the round-trip note '{note}'; body was: {body}")
    });

    let identifier = cloud_event
        .pointer("/data/identifier")
        .and_then(Value::as_object)
        .unwrap_or_else(|| panic!("CloudEvent must have data.identifier; got: {cloud_event}"));
    assert_eq!(
        identifier.get("polygon").and_then(Value::as_str),
        Some(producer_polygon),
        "round-trip bug regression: producer-sent polygon must appear in data.identifier.polygon \
         on the emitted CloudEvent. Got identifier: {identifier:?}"
    );
    assert_eq!(
        identifier.get("date").and_then(Value::as_str),
        Some("20250706"),
        "topic-derived identifier fields must still be present"
    );
    assert_eq!(
        identifier.get("time").and_then(Value::as_str),
        Some("1200"),
        "topic-derived identifier fields must still be present"
    );
}

#[tokio::test]
async fn live_watch_emits_cloudevent_carrying_the_producer_polygon_in_data_identifier() {
    let app = spawn_streaming_test_app().await;
    let client = reqwest::Client::new();
    let suffix = unique_suffix();
    let note = format!("POLYGON_ROUNDTRIP_LIVE_{suffix}");
    let producer_polygon = "(50.0,10.0,52.0,10.0,52.0,12.0,50.0,12.0,50.0,10.0)";

    let mut watch_response = client
        .post(format!("{}/api/v1/watch", &app.address))
        .header("Content-Type", "application/json")
        .json(&json!({
            "event_type": "test_polygon",
            "identifier": {
                "time": "1200",
                "polygon": "(49.0,9.0,53.0,9.0,53.0,13.0,49.0,13.0,49.0,9.0)",
            }
        }))
        .send()
        .await
        .expect("failed to open watch stream");
    assert_eq!(watch_response.status(), StatusCode::OK);

    sleep(Duration::from_millis(100)).await;
    let publish =
        post_test_polygon_notification_with_polygon(&client, &app.address, &note, producer_polygon)
            .await;
    assert_eq!(publish.status(), StatusCode::OK);

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut observed = String::new();
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let chunk = match timeout(remaining, watch_response.chunk()).await {
            Err(_) => break,
            Ok(Ok(Some(c))) => c,
            Ok(Ok(None)) => break,
            Ok(Err(e)) => panic!("watch stream read failed: {e}"),
        };
        observed.push_str(&String::from_utf8_lossy(&chunk));
        if observed.contains(&note) {
            break;
        }
    }

    let cloud_event = extract_cloud_event_matching(&observed, &note).unwrap_or_else(|| {
        panic!("live watch must deliver a CloudEvent for note '{note}'; observed: {observed}")
    });
    let identifier = cloud_event
        .pointer("/data/identifier")
        .and_then(Value::as_object)
        .unwrap_or_else(|| panic!("CloudEvent must have data.identifier; got: {cloud_event}"));
    assert_eq!(
        identifier.get("polygon").and_then(Value::as_str),
        Some(producer_polygon),
        "live watch must carry the producer polygon in data.identifier.polygon; \
         got identifier: {identifier:?}"
    );
}

/// Scan an SSE response body for the first CloudEvent of type
/// `int.ecmwf.aviso.test_polygon` whose serialized payload contains `marker`.
/// Returns the parsed CloudEvent JSON, or `None` if no such event is present.
fn extract_cloud_event_matching(sse_body: &str, marker: &str) -> Option<Value> {
    sse_body
        .lines()
        .filter_map(|line| line.strip_prefix("data:").map(str::trim))
        .filter(|payload| payload.contains(marker))
        .filter_map(|payload| serde_json::from_str::<Value>(payload).ok())
        .find(|parsed| {
            parsed.get("type").and_then(Value::as_str) == Some("int.ecmwf.aviso.test_polygon")
        })
}
