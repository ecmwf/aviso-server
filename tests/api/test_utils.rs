use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before unix epoch")
        .as_nanos();
    nanos.to_string()
}

pub fn test_polygon() -> &'static str {
    "(52.5,13.4,52.6,13.5,52.5,13.6,52.4,13.5,52.5,13.4)"
}

pub async fn post_test_polygon_notification(
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
