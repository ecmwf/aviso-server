// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use aviso_server::configuration::Settings;
use aviso_server::notification_backend::build_backend;
use reqwest::{Client, StatusCode};
use serde_json::{Value, json};

#[path = "common/nats.rs"]
mod nats;
#[path = "common/replay_app.rs"]
mod replay_app;

use nats::{nats_tests_enabled, nats_url};
use replay_app::{ReplayApp, sse_data_events};

#[tokio::test]
async fn default_cap_uses_10001st_notification_only_as_lookahead() {
    let base = format!("DefaultQuota_{}", uuid::Uuid::new_v4().simple());
    let mut config: Settings = serde_json::from_value(json!({
        "application": {"host": "127.0.0.1", "port": 0, "base_url": "http://localhost"},
        "notification_backend": {"kind": "in_memory"},
        "notification_schema": {"default_quota": {
            "topic": {"base": base, "key_order": ["number"]},
            "identifier": {"number": {"type": "IntHandler", "required": true}}
        }}
    }))
    .unwrap();
    assert_eq!(config.watch_endpoint.max_historical_notifications, 10_000);
    assert_eq!(config.watch_endpoint.replay_batch_size, 100);
    config.watch_endpoint.replay_batch_delay_ms = 0;
    config.init_global_config();
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap();
    for kind in ["in_memory", "jetstream"] {
        if kind == "jetstream" && !nats_tests_enabled() {
            continue;
        }
        config.notification_backend = serde_json::from_value(json!({
            "kind": kind, "in_memory": {"max_topics": 20000},
            "jetstream": {"nats_url": nats_url()}
        }))
        .unwrap();
        let backend = build_backend(&config.notification_backend).await.unwrap();
        for number in 1..=10_001 {
            backend
                .put_messages(
                    &format!("{base}.{number}"),
                    json!({"number": number}).to_string(),
                )
                .await
                .unwrap();
        }
        let app = ReplayApp::start(&config, backend.clone());
        let address = &app.address;
        for (cursor, expected, truncated) in [
            (json!({"from_id": "0"}), 10_000, true),
            (json!({"from_id": "2"}), 10_000, false),
            (json!({"from_id": "3"}), 9_999, false),
            (json!({"from_id": "10002"}), 0, false),
            (json!({"from_date": "2000-01-01T00:00:00Z"}), 10_000, true),
            (json!({"from_date": "2100-01-01T00:00:00Z"}), 0, false),
        ] {
            for endpoint in if truncated {
                vec!["replay", "watch"]
            } else {
                vec!["replay"]
            } {
                let mut request = cursor.clone();
                request["event_type"] = json!("default_quota");
                request["identifier"] = json!({"number": {"gte": 0}});
                let response = client
                    .post(format!("{address}/{endpoint}"))
                    .json(&request)
                    .send()
                    .await
                    .unwrap();
                let status = response.status();
                let body = response.text().await.unwrap();
                assert_eq!(status, StatusCode::OK, "{kind}: {request}: {body}");
                assert_eq!(
                    body.matches("event: replay\n").count(),
                    expected,
                    "{kind}: {request}"
                );
                assert!(!body.contains("event: error"));
                assert!(!body.contains("event: live-notification"));
                let events: Vec<Value> = sse_data_events(&body);
                assert_eq!(events[0]["batch_size"], 100);
                let limits: Vec<_> = events
                    .iter()
                    .filter(|event| event["type"] == "notification_replay_limit_reached")
                    .collect();
                assert_eq!(limits.len(), usize::from(truncated), "{kind}: {request}");
                if truncated {
                    assert_eq!(limits[0]["max_allowed"], 10_000);
                    assert_eq!(
                        events[events.len() - 3]["data"]["payload"]["number"],
                        10_000
                    );
                }
                assert_eq!(
                    body.matches("replay_completed").count(),
                    usize::from(!truncated)
                );
                assert_eq!(events.last().unwrap()["reason"], "end_of_stream");
            }
        }
        app.stop().await;
        backend.wipe_stream(&base.to_uppercase()).await.unwrap();
    }
}
