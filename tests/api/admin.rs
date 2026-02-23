use crate::helpers::spawn_streaming_test_app;
use crate::test_utils::{post_mars_notification, unique_suffix};
use serde_json::json;

fn extract_event_sequence(replay_body: &str, event_type: &str) -> Option<u64> {
    let marker = format!("\"id\":\"{}@", event_type);
    let id_start = replay_body.find(&marker)?;
    let id_tail = &replay_body[id_start + marker.len()..];
    let id_end = id_tail.find('"')?;
    id_tail[..id_end].parse::<u64>().ok()
}

fn replay_event_count(replay_body: &str) -> usize {
    replay_body.matches("\nevent: replay\ndata: ").count()
}

#[tokio::test]
async fn delete_notification_rejects_invalid_id_format() {
    let app = spawn_streaming_test_app().await;
    let response = reqwest::Client::new()
        .delete(format!(
            "{}/api/v1/admin/notification/not-a-valid-id",
            app.address
        ))
        .send()
        .await
        .expect("failed to send delete request");

    assert_eq!(response.status().as_u16(), 400);
}

#[tokio::test]
async fn delete_notification_returns_not_found_for_missing_sequence() {
    let app = spawn_streaming_test_app().await;
    let response = reqwest::Client::new()
        .delete(format!(
            "{}/api/v1/admin/notification/test_polygon@999999999",
            app.address
        ))
        .send()
        .await
        .expect("failed to send delete request");

    assert_eq!(response.status().as_u16(), 404);
}

#[tokio::test]
async fn delete_notification_accepts_event_type_alias_and_removes_message() {
    let app = spawn_streaming_test_app().await;
    let client = reqwest::Client::new();
    let unique_stream = format!("ens.member.{}", unique_suffix());
    let expected_note = format!("admin-delete-note-{}", unique_suffix());

    let publish_response =
        post_mars_notification(&client, &app.address, &expected_note, &unique_stream).await;
    assert_eq!(publish_response.status().as_u16(), 200);

    let replay_before = client
        .post(format!("{}/api/v1/replay", app.address))
        .header("Content-Type", "application/json")
        .json(&json!({
            "event_type": "mars",
            "identifier": {
                "class": "od",
                "expver": "0001",
                "domain": "g",
                "date": "20250706",
                "time": "1200",
                "stream": unique_stream,
                "step": "1"
            },
            "from_id": "1"
        }))
        .send()
        .await
        .expect("failed to send replay request");
    assert_eq!(replay_before.status().as_u16(), 200);
    let before_body = replay_before
        .text()
        .await
        .expect("failed to read replay body before delete");
    assert!(
        replay_event_count(&before_body) == 1,
        "expected replay to include exactly one historical message before delete; body: {before_body}"
    );

    let sequence = extract_event_sequence(&before_body, "mars")
        .expect("expected replay output to include mars sequence in CloudEvent id");
    let delete_response = client
        .delete(format!(
            "{}/api/v1/admin/notification/mars@{}",
            app.address, sequence
        ))
        .send()
        .await
        .expect("failed to send delete request");
    assert_eq!(delete_response.status().as_u16(), 200);

    let replay_after = client
        .post(format!("{}/api/v1/replay", app.address))
        .header("Content-Type", "application/json")
        .json(&json!({
            "event_type": "mars",
            "identifier": {
                "class": "od",
                "expver": "0001",
                "domain": "g",
                "date": "20250706",
                "time": "1200",
                "stream": unique_stream,
                "step": "1"
            },
            "from_id": "1"
        }))
        .send()
        .await
        .expect("failed to send replay request after delete");
    assert_eq!(replay_after.status().as_u16(), 200);
    let after_body = replay_after
        .text()
        .await
        .expect("failed to read replay body after delete");
    assert!(
        replay_event_count(&after_body) == 0,
        "expected replay to exclude deleted message; body: {after_body}"
    );
}
