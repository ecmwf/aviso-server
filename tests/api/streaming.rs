use crate::helpers::spawn_streaming_test_app;
use reqwest::StatusCode;
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::{Duration, Instant, sleep};

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before unix epoch")
        .as_nanos();
    nanos.to_string()
}

fn test_polygon() -> &'static str {
    "(52.5,13.4,52.6,13.5,52.5,13.6,52.4,13.5,52.5,13.4)"
}

async fn post_test_polygon_notification(
    client: &reqwest::Client,
    base_url: &str,
    note: &str,
) -> reqwest::Response {
    client
        .post(format!("{}/api/v1/notification", base_url))
        .header("Content-Type", "application/json")
        .json(&json!({
            "event_type": "test_polygon",
            "identifier": {
                "date": "20250706",
                "time": "1200",
                "polygon": test_polygon(),
            },
            "payload": {
                "note": note,
            }
        }))
        .send()
        .await
        .expect("failed to send notification")
}

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

    sleep(Duration::from_millis(20)).await;
    let from_date = chrono::Utc::now().to_rfc3339();
    sleep(Duration::from_millis(20)).await;

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
