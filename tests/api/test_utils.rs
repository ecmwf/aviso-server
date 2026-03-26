// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

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

pub fn outside_polygon() -> &'static str {
    "(10.0,10.0,10.2,10.0,10.2,10.2,10.0,10.2,10.0,10.0)"
}

pub async fn post_test_polygon_notification(
    client: &reqwest::Client,
    base_url: &str,
    note: &str,
) -> reqwest::Response {
    post_polygon_notification(client, base_url, "test_polygon", note, test_polygon()).await
}

pub async fn post_polygon_notification_for_event_with_identifier(
    client: &reqwest::Client,
    base_url: &str,
    event_type: &str,
    note: &str,
    polygon: &str,
    date: &str,
    time: &str,
) -> reqwest::Response {
    post_polygon_notification_with_identifier(
        client, base_url, event_type, note, polygon, date, time,
    )
    .await
}

pub async fn post_test_polygon_notification_with_polygon(
    client: &reqwest::Client,
    base_url: &str,
    note: &str,
    polygon: &str,
) -> reqwest::Response {
    post_polygon_notification(client, base_url, "test_polygon", note, polygon).await
}

pub async fn post_test_polygon_optional_notification_with_polygon(
    client: &reqwest::Client,
    base_url: &str,
    note: &str,
    polygon: &str,
) -> reqwest::Response {
    post_polygon_notification(client, base_url, "test_polygon_optional", note, polygon).await
}

async fn post_polygon_notification(
    client: &reqwest::Client,
    base_url: &str,
    event_type: &str,
    note: &str,
    polygon: &str,
) -> reqwest::Response {
    post_polygon_notification_with_identifier(
        client, base_url, event_type, note, polygon, "20250706", "1200",
    )
    .await
}

async fn post_polygon_notification_with_identifier(
    client: &reqwest::Client,
    base_url: &str,
    event_type: &str,
    note: &str,
    polygon: &str,
    date: &str,
    time: &str,
) -> reqwest::Response {
    client
        .post(format!("{}/api/v1/notification", base_url))
        .header("Content-Type", "application/json")
        .json(&json!({
            "event_type": event_type,
            "identifier": {
                "date": date,
                "time": time,
                "polygon": polygon,
            },
            "payload": {
                "note": note,
            }
        }))
        .send()
        .await
        .expect("failed to send notification")
}

pub async fn post_mars_notification(
    client: &reqwest::Client,
    base_url: &str,
    note: &str,
    stream_value: &str,
) -> reqwest::Response {
    post_mars_notification_with_identifier(client, base_url, note, stream_value, "1", "g").await
}

pub async fn post_mars_notification_with_identifier(
    client: &reqwest::Client,
    base_url: &str,
    note: &str,
    stream_value: &str,
    step: &str,
    domain: &str,
) -> reqwest::Response {
    client
        .post(format!("{}/api/v1/notification", base_url))
        .header("Content-Type", "application/json")
        .json(&json!({
            "event_type": "mars",
            "identifier": {
                "class": "od",
                "expver": "0001",
                "domain": domain,
                "date": "20250706",
                "time": "1200",
                "stream": stream_value,
                "step": step
            },
            "payload": note
        }))
        .send()
        .await
        .expect("failed to send mars notification")
}

pub async fn post_dissemination_notification(
    client: &reqwest::Client,
    base_url: &str,
    note: &str,
    target_value: &str,
) -> reqwest::Response {
    client
        .post(format!("{}/api/v1/notification", base_url))
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
            "payload": {
                "note": note
            }
        }))
        .send()
        .await
        .expect("failed to send dissemination notification")
}

pub async fn post_extreme_event_notification_with_identifier(
    client: &reqwest::Client,
    base_url: &str,
    note: &str,
    region: &str,
    severity: i64,
    anomaly: f64,
) -> reqwest::Response {
    client
        .post(format!("{}/api/v1/notification", base_url))
        .header("Content-Type", "application/json")
        .json(&json!({
            "event_type": "extreme",
            "identifier": {
                "region": region,
                "run_time": "1200",
                "severity": severity,
                "anomaly": anomaly
            },
            "payload": {
                "note": note
            }
        }))
        .send()
        .await
        .expect("failed to send extreme_event notification")
}
