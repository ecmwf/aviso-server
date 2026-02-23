use crate::helpers::spawn_streaming_test_app;
use crate::test_utils::{
    outside_polygon, post_dissemination_notification, post_mars_notification,
    post_test_polygon_notification, post_test_polygon_notification_with_polygon,
    post_test_polygon_optional_notification_with_polygon, test_polygon, unique_suffix,
};
use reqwest::StatusCode;
use serde_json::json;
use tokio::time::{Duration, Instant, sleep};

#[tokio::test]
async fn watch_without_replay_params_is_live_only() {
    let app = spawn_streaming_test_app().await;
    let client = reqwest::Client::new();
    let suffix = unique_suffix();

    let historical_note = format!("HISTORICAL_BEFORE_WATCH_{suffix}");
    let live_note = format!("LIVE_AFTER_WATCH_{suffix}");

    let historical_response =
        post_test_polygon_notification(&client, &app.address, &historical_note).await;
    assert_eq!(historical_response.status(), StatusCode::OK);

    let mut watch_response = client
        .post(format!("{}/api/v1/watch", &app.address))
        .header("Content-Type", "application/json")
        .json(&json!({
            "event_type": "test_polygon",
            "identifier": {
                "time": "1200",
                "polygon": test_polygon(),
            }
        }))
        .send()
        .await
        .expect("failed to call watch endpoint");

    assert_eq!(watch_response.status(), StatusCode::OK);

    sleep(Duration::from_millis(100)).await;
    let live_response = post_test_polygon_notification(&client, &app.address, &live_note).await;
    assert_eq!(live_response.status(), StatusCode::OK);

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut observed = String::new();
    let mut saw_live_note = false;

    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let next_chunk_result = tokio::time::timeout(remaining, watch_response.chunk()).await;
        let next_chunk = match next_chunk_result {
            Err(_) => break,
            Ok(chunk_result) => chunk_result.expect("failed to read watch response chunk"),
        };

        match next_chunk {
            Some(chunk) => {
                observed.push_str(&String::from_utf8_lossy(&chunk));
                if observed.contains(&live_note) {
                    saw_live_note = true;
                    break;
                }
            }
            None => break,
        }
    }

    assert!(
        saw_live_note,
        "expected watch stream to include live note: {live_note}; observed: {observed}"
    );
    assert!(
        !observed.contains(&historical_note),
        "expected watch stream to exclude historical note: {historical_note}; observed: {observed}"
    );
}

#[tokio::test]
async fn replay_with_from_date_excludes_older_messages() {
    let app = spawn_streaming_test_app().await;
    let client = reqwest::Client::new();
    let suffix = unique_suffix();

    let old_note = format!("OLD_BEFORE_FROM_DATE_{suffix}");
    let new_note = format!("NEW_AFTER_FROM_DATE_{suffix}");

    let old_response = post_test_polygon_notification(&client, &app.address, &old_note).await;
    assert_eq!(old_response.status(), StatusCode::OK);

    sleep(Duration::from_millis(100)).await;
    let from_date = chrono::Utc::now().to_rfc3339();
    sleep(Duration::from_millis(100)).await;

    let new_response = post_test_polygon_notification(&client, &app.address, &new_note).await;
    assert_eq!(new_response.status(), StatusCode::OK);

    let replay_response = client
        .post(format!("{}/api/v1/replay", &app.address))
        .header("Content-Type", "application/json")
        .json(&json!({
            "event_type": "test_polygon",
            "identifier": {
                "time": "1200",
                "polygon": test_polygon(),
            },
            "from_date": from_date,
        }))
        .send()
        .await
        .expect("failed to call replay endpoint");

    assert_eq!(replay_response.status(), StatusCode::OK);
    let body = replay_response
        .text()
        .await
        .expect("failed to read replay response body");

    assert!(
        body.contains(&new_note),
        "expected replay to include new message note: {new_note}; body: {body}"
    );
    assert!(
        !body.contains(&old_note),
        "expected replay to exclude old message note: {old_note}; body: {body}"
    );
}

#[tokio::test]
async fn replay_with_from_id_returns_messages() {
    let app = spawn_streaming_test_app().await;
    let client = reqwest::Client::new();
    let suffix = unique_suffix();

    let first_note = format!("REPLAY_ID_FIRST_{suffix}");
    let second_note = format!("REPLAY_ID_SECOND_{suffix}");

    let first_response = post_test_polygon_notification(&client, &app.address, &first_note).await;
    assert_eq!(first_response.status(), StatusCode::OK);
    let second_response = post_test_polygon_notification(&client, &app.address, &second_note).await;
    assert_eq!(second_response.status(), StatusCode::OK);

    let replay_response = client
        .post(format!("{}/api/v1/replay", &app.address))
        .header("Content-Type", "application/json")
        .json(&json!({
            "event_type": "test_polygon",
            "identifier": {
                "time": "1200",
                "polygon": test_polygon(),
            },
            "from_id": "1",
        }))
        .send()
        .await
        .expect("failed to call replay endpoint");

    assert_eq!(replay_response.status(), StatusCode::OK);
    let body = replay_response
        .text()
        .await
        .expect("failed to read replay response body");

    assert!(
        body.contains(&first_note) || body.contains(&second_note),
        "expected replay to include historical notifications; body: {body}"
    );
}

#[tokio::test]
async fn replay_without_from_id_or_from_date_returns_bad_request() {
    let app = spawn_streaming_test_app().await;
    let client = reqwest::Client::new();

    let response = client
        .post(format!("{}/api/v1/replay", &app.address))
        .header("Content-Type", "application/json")
        .json(&json!({
            "event_type": "test_polygon",
            "identifier": {
                "time": "1200",
                "polygon": test_polygon(),
            }
        }))
        .send()
        .await
        .expect("failed to call replay endpoint");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn watch_rejects_polygon_and_point_together() {
    let app = spawn_streaming_test_app().await;
    let client = reqwest::Client::new();

    let response = client
        .post(format!("{}/api/v1/watch", &app.address))
        .header("Content-Type", "application/json")
        .json(&json!({
            "event_type": "test_polygon",
            "identifier": {
                "time": "1200",
                "polygon": test_polygon(),
            },
            "point": "52.55,13.5",
        }))
        .send()
        .await
        .expect("failed to call watch endpoint");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn replay_rejects_polygon_and_point_together() {
    let app = spawn_streaming_test_app().await;
    let client = reqwest::Client::new();

    let response = client
        .post(format!("{}/api/v1/replay", &app.address))
        .header("Content-Type", "application/json")
        .json(&json!({
            "event_type": "test_polygon",
            "identifier": {
                "time": "1200",
                "polygon": test_polygon(),
            },
            "point": "52.55,13.5",
            "from_id": "1",
        }))
        .send()
        .await
        .expect("failed to call replay endpoint");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn replay_rejects_invalid_point_format() {
    let app = spawn_streaming_test_app().await;
    let client = reqwest::Client::new();

    let response = client
        .post(format!("{}/api/v1/replay", &app.address))
        .header("Content-Type", "application/json")
        .json(&json!({
            "event_type": "test_polygon",
            "identifier": {
                "time": "1200"
            },
            "point": "invalid-point",
            "from_id": "1",
        }))
        .send()
        .await
        .expect("failed to call replay endpoint");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn replay_with_point_matches_only_containing_polygons() {
    let app = spawn_streaming_test_app().await;
    let client = reqwest::Client::new();
    let suffix = unique_suffix();

    let inside_note = format!("POINT_INSIDE_{suffix}");
    let outside_note = format!("POINT_OUTSIDE_{suffix}");

    let inside_response = post_test_polygon_notification_with_polygon(
        &client,
        &app.address,
        &inside_note,
        test_polygon(),
    )
    .await;
    assert_eq!(inside_response.status(), StatusCode::OK);

    let outside_response = post_test_polygon_notification_with_polygon(
        &client,
        &app.address,
        &outside_note,
        outside_polygon(),
    )
    .await;
    assert_eq!(outside_response.status(), StatusCode::OK);

    let replay_response = client
        .post(format!("{}/api/v1/replay", &app.address))
        .header("Content-Type", "application/json")
        .json(&json!({
            "event_type": "test_polygon",
            "identifier": {
                "time": "1200"
            },
            "point": "52.55,13.5",
            "from_id": "1",
        }))
        .send()
        .await
        .expect("failed to call replay endpoint");

    assert_eq!(replay_response.status(), StatusCode::OK);
    let body = replay_response
        .text()
        .await
        .expect("failed to read replay response body");

    assert!(
        body.contains(&inside_note),
        "expected replay to include message whose polygon contains point; body: {body}"
    );
    assert!(
        !body.contains(&outside_note),
        "expected replay to exclude message whose polygon does not contain point; body: {body}"
    );
}

#[tokio::test]
async fn replay_optional_polygon_without_polygon_or_point_matches_by_other_identifiers() {
    let app = spawn_streaming_test_app().await;
    let client = reqwest::Client::new();
    let suffix = unique_suffix();

    let first_note = format!("OPTIONAL_POLYGON_FIRST_{suffix}");
    let second_note = format!("OPTIONAL_POLYGON_SECOND_{suffix}");

    let first_response = post_test_polygon_optional_notification_with_polygon(
        &client,
        &app.address,
        &first_note,
        test_polygon(),
    )
    .await;
    assert_eq!(first_response.status(), StatusCode::OK);

    let second_response = post_test_polygon_optional_notification_with_polygon(
        &client,
        &app.address,
        &second_note,
        outside_polygon(),
    )
    .await;
    assert_eq!(second_response.status(), StatusCode::OK);

    let replay_response = client
        .post(format!("{}/api/v1/replay", &app.address))
        .header("Content-Type", "application/json")
        .json(&json!({
            "event_type": "test_polygon_optional",
            "identifier": {
                "time": "1200"
            },
            "from_id": "1",
        }))
        .send()
        .await
        .expect("failed to call replay endpoint");

    assert_eq!(replay_response.status(), StatusCode::OK);
    let body = replay_response
        .text()
        .await
        .expect("failed to read replay response body");

    assert!(
        body.contains(&first_note),
        "expected replay to include first note without spatial filters; body: {body}"
    );
    assert!(
        body.contains(&second_note),
        "expected replay to include second note without spatial filters; body: {body}"
    );
}

#[tokio::test]
async fn watch_optional_polygon_without_polygon_or_point_receives_live_notifications() {
    let app = spawn_streaming_test_app().await;
    let client = reqwest::Client::new();
    let suffix = unique_suffix();
    let live_note = format!("OPTIONAL_POLYGON_WATCH_LIVE_{suffix}");

    let mut watch_response = client
        .post(format!("{}/api/v1/watch", &app.address))
        .header("Content-Type", "application/json")
        .json(&json!({
            "event_type": "test_polygon_optional",
            "identifier": {
                "time": "1200"
            }
        }))
        .send()
        .await
        .expect("failed to call watch endpoint");

    assert_eq!(watch_response.status(), StatusCode::OK);

    sleep(Duration::from_millis(100)).await;
    let live_response = post_test_polygon_optional_notification_with_polygon(
        &client,
        &app.address,
        &live_note,
        outside_polygon(),
    )
    .await;
    assert_eq!(live_response.status(), StatusCode::OK);

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut observed = String::new();
    let mut saw_live_note = false;

    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let next_chunk_result = tokio::time::timeout(remaining, watch_response.chunk()).await;
        let next_chunk = match next_chunk_result {
            Err(_) => break,
            Ok(chunk_result) => chunk_result.expect("failed to read watch response chunk"),
        };

        match next_chunk {
            Some(chunk) => {
                observed.push_str(&String::from_utf8_lossy(&chunk));
                if observed.contains(&live_note) {
                    saw_live_note = true;
                    break;
                }
            }
            None => break,
        }
    }

    assert!(
        saw_live_note,
        "expected watch stream to include live note without spatial filters: {live_note}; observed: {observed}"
    );
}

#[tokio::test]
async fn replay_with_from_id_returns_mars_messages_with_dot_values() {
    let app = spawn_streaming_test_app().await;
    let client = reqwest::Client::new();
    let suffix = unique_suffix();

    let first_note = format!("MARS_REPLAY_FIRST_{suffix}");
    let second_note = format!("MARS_REPLAY_SECOND_{suffix}");
    let stream_value = format!("ens.member.{suffix}");

    let first_response =
        post_mars_notification(&client, &app.address, &first_note, &stream_value).await;
    assert_eq!(first_response.status(), StatusCode::OK);
    let second_response =
        post_mars_notification(&client, &app.address, &second_note, &stream_value).await;
    assert_eq!(second_response.status(), StatusCode::OK);

    let replay_response = client
        .post(format!("{}/api/v1/replay", &app.address))
        .header("Content-Type", "application/json")
        .json(&json!({
            "event_type": "mars",
            "identifier": {
                "class": "od",
                "expver": "0001",
                "domain": "g",
                "date": "20250706",
                "time": "1200",
                "stream": stream_value,
                "step": "1"
            },
            "from_id": "1",
        }))
        .send()
        .await
        .expect("failed to call replay endpoint");

    assert_eq!(replay_response.status(), StatusCode::OK);
    let body = replay_response
        .text()
        .await
        .expect("failed to read replay response body");

    assert!(
        body.contains(&stream_value),
        "expected replay to include mars identifier stream with dot value: {stream_value}; body: {body}"
    );
}

#[tokio::test]
async fn watch_with_from_date_replays_dissemination_with_dot_target_then_goes_live() {
    let app = spawn_streaming_test_app().await;
    let client = reqwest::Client::new();
    let suffix = unique_suffix();
    let target_value = format!("target.v1.{suffix}");

    let historical_note = format!("DISS_HISTORICAL_BEFORE_{suffix}");
    let live_note = format!("DISS_LIVE_AFTER_{suffix}");

    let old_response =
        post_dissemination_notification(&client, &app.address, &historical_note, &target_value)
            .await;
    assert_eq!(old_response.status(), StatusCode::OK);

    sleep(Duration::from_millis(100)).await;
    let from_date = chrono::Utc::now().to_rfc3339();
    sleep(Duration::from_millis(100)).await;

    let mut watch_response = client
        .post(format!("{}/api/v1/watch", &app.address))
        .header("Content-Type", "application/json")
        .json(&json!({
            "event_type": "dissemination",
            "identifier": {
                "destination": "FOO",
                "target": target_value,
                "class": "od",
                "expver": "0001",
                "domain": "g",
                "date": "20250706",
                "time": "1200",
                "stream": "enfo",
                "step": "1"
            },
            "from_date": from_date,
        }))
        .send()
        .await
        .expect("failed to call watch endpoint");

    assert_eq!(watch_response.status(), StatusCode::OK);

    let live_response =
        post_dissemination_notification(&client, &app.address, &live_note, &target_value).await;
    assert_eq!(live_response.status(), StatusCode::OK);

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut observed = String::new();
    let mut saw_live_note = false;
    let mut saw_historical_note = false;

    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let next_chunk_result = tokio::time::timeout(remaining, watch_response.chunk()).await;
        let next_chunk = match next_chunk_result {
            Err(_) => break,
            Ok(chunk_result) => chunk_result.expect("failed to read watch response chunk"),
        };

        match next_chunk {
            Some(chunk) => {
                observed.push_str(&String::from_utf8_lossy(&chunk));
                if observed.contains(&historical_note) {
                    saw_historical_note = true;
                }
                if observed.contains(&live_note) {
                    saw_live_note = true;
                    break;
                }
            }
            None => break,
        }
    }

    assert!(
        !saw_historical_note,
        "expected from_date watch to exclude older diss message; observed: {observed}"
    );
    assert!(
        saw_live_note,
        "expected from_date watch to include live diss message; observed: {observed}"
    );
}
