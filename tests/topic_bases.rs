// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use aviso_server::configuration::Settings;
use aviso_server::notification_backend::jetstream::{
    backend::JetStreamBackend, config::JetStreamConfig,
};
use aviso_server::notification_backend::{NotificationBackend, replay::BatchParams};
use aviso_server::startup::Application;
use futures_util::StreamExt;
use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

const INVALID_BASES: &[&str] = &["", "a.b", "a%b", "a*b", "a>b", "a b", "\u{e9}", "_a", "-a"];

fn settings() -> Settings {
    serde_json::from_value(json!({
        "application": {"host": "127.0.0.1", "port": 0,
            "base_url": "http://localhost", "static_files_path": "./src/static"},
        "notification_backend": {"kind": "in_memory"},
        "notification_schema_strict": false,
        "notification_schema": {"weather": {
            "topic": {"base": "weather", "key_order": ["number", "text"]},
            "identifier": {
                "number": {"type": "FloatHandler", "required": true},
                "text": {"type": "StringHandler", "required": true}
            }
        }}
    }))
    .unwrap()
}

#[tokio::test]
async fn startup_rejects_invalid_and_case_colliding_bases() {
    for base in INVALID_BASES {
        for with_topic in [true, false] {
            let mut config = settings();
            let schemas = config.notification_schema.as_mut().unwrap();
            let mut schema = schemas.remove("weather").unwrap();
            if with_topic {
                schema.topic.as_mut().unwrap().base = (*base).into();
                schemas.insert("weather".into(), schema);
            } else {
                schema.topic = None;
                schemas.insert((*base).into(), schema);
            }
            let error = Application::build(config, CancellationToken::new())
                .await
                .err()
                .expect("startup must reject the base before opening any connection");
            assert!(error.to_string().contains("invalid topic base"), "{error}");
        }
    }
    for with_topic in [true, false] {
        let mut config = settings();
        let schemas = config.notification_schema.as_mut().unwrap();
        let mut duplicate = schemas["weather"].clone();
        if with_topic {
            duplicate.topic.as_mut().unwrap().base = "WEATHER".into();
        } else {
            duplicate.topic = None;
        }
        schemas.insert("WEATHER".into(), duplicate);
        let error = Application::build(config, CancellationToken::new())
            .await
            .err()
            .expect("case-insensitive collision must fail startup");
        assert!(error.to_string().contains("both define topic base"));
    }
}

fn events(body: &str) -> Vec<Value> {
    body.lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .filter_map(|data| serde_json::from_str::<Value>(data.trim()).ok())
        .filter(|event| event.get("specversion").is_some())
        .collect()
}

#[tokio::test]
async fn jetstream_base_targets_and_identifier_codec() {
    if !std::env::var("AVISO_RUN_NATS_TESTS")
        .is_ok_and(|value| value == "1" || value.eq_ignore_ascii_case("true"))
    {
        return;
    }
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let bases = [
        format!("Simple{suffix}"),
        format!("Under_score{suffix}"),
        format!("Hyphen-base{suffix}"),
        format!("UPPER{}", suffix.to_ascii_uppercase()),
    ];
    let mut config = settings();
    let schemas = config.notification_schema.as_mut().unwrap();
    let template = schemas.remove("weather").unwrap();
    for (index, base) in bases.iter().enumerate() {
        let mut schema = template.clone();
        schema.topic.as_mut().unwrap().base = base.clone();
        schema.storage_policy =
            Some(serde_json::from_value(json!({"allow_duplicates": true})).unwrap());
        schemas.insert(format!("event{index}"), schema);
    }
    // Also exercise a configured event without a topic and the no-schema fallback.
    let mut generic = template;
    generic.topic = None;
    schemas.insert(format!("Generic_{suffix}"), generic);
    let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    config.notification_backend = serde_json::from_value(json!({
        "kind": "jetstream", "jetstream": {"nats_url": nats_url}
    }))
    .unwrap();
    let backend_config =
        JetStreamConfig::from_backend_settings(&config.notification_backend).unwrap();
    config.init_global_config();
    let shutdown = CancellationToken::new();
    let app = Application::build(config, shutdown.clone()).await.unwrap();
    let address = format!("http://127.0.0.1:{}", app.port());
    let server = tokio::spawn(app.run_until_stopped());
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap();
    let nats = async_nats::connect(nats_url).await.unwrap();
    let js = async_nats::jetstream::new(nats);
    let backend = JetStreamBackend::new(backend_config).await.unwrap();

    for base in INVALID_BASES {
        for (endpoint, code) in [
            ("notification", "INVALID_NOTIFICATION_REQUEST"),
            ("replay", "INVALID_REPLAY_REQUEST"),
            ("watch", "INVALID_WATCH_REQUEST"),
        ] {
            let response = client
                .post(format!("{address}/api/v1/{endpoint}"))
                .json(&json!({"event_type": base, "identifier": {"text": "a.b"}, "from_id": "0"}))
                .send()
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::BAD_REQUEST,
                "{base:?} {endpoint}"
            );
            assert_eq!(response.json::<Value>().await.unwrap()["code"], code);
        }
    }

    for (index, base) in bases.iter().enumerate() {
        let event_type = format!("event{index}");
        let identifier = json!({"number": 1.45, "text": "a.b%2Ec"});
        for number in [1.45, 9.75] {
            let response = client
                .post(format!("{address}/api/v1/notification"))
                .json(&json!({"event_type": event_type,
                    "identifier": {"number": number, "text": "a.b%2Ec"},
                    "payload": {"number": number}}))
                .send()
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::OK,
                "{}",
                response.text().await.unwrap()
            );
        }
        let name = base.to_ascii_uppercase();
        let mut stream = js.get_stream(&name).await.unwrap();
        assert_eq!(stream.cached_info().config.subjects, [format!("{base}.>")]);
        assert_eq!(stream.cached_info().state.messages, 2);
        let raw = stream.get_raw_message(1).await.unwrap();
        assert_eq!(raw.subject.as_str(), format!("{base}.1%2E45.a%2Eb%252Ec"));

        // Exact values and a numeric constraint must both select only the first value.
        for filter in [
            identifier.clone(),
            json!({"number": {"between": [1.0, 2.0]}, "text": "a.b%2Ec"}),
        ] {
            let response = client
                .post(format!("{address}/api/v1/replay"))
                .json(&json!({"event_type": event_type, "identifier": filter, "from_id": "0"}))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body = response.text().await.unwrap();
            let replay = events(&body);
            assert_eq!(replay.len(), 1, "{body}");
            assert_eq!(replay[0]["data"]["identifier"]["number"], "1.45");
            assert_eq!(replay[0]["data"]["identifier"]["text"], "a.b%2Ec");
        }
        let mut watch = client
            .post(format!("{address}/api/v1/watch"))
            .json(&json!({"event_type": event_type,
                "identifier": {"number": {"between": [1.0, 2.0]}, "text": "a.b%2Ec"}}))
            .send()
            .await
            .unwrap();
        assert_eq!(watch.status(), StatusCode::OK);
        for number in [9.75, 1.45] {
            let response = client
                .post(format!("{address}/api/v1/notification"))
                .json(&json!({"event_type": event_type,
                    "identifier": {"number": number, "text": "a.b%2Ec"},
                    "payload": {"live": true}}))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        }
        let mut body = String::new();
        tokio::time::timeout(Duration::from_secs(10), async {
            while events(&body).is_empty() {
                body.push_str(std::str::from_utf8(&watch.chunk().await.unwrap().unwrap()).unwrap());
            }
        })
        .await
        .unwrap();
        let live = events(&body);
        assert_eq!(live.len(), 1, "{body}");
        assert_eq!(live[0]["data"]["identifier"]["number"], "1.45");
        assert_eq!(live[0]["data"]["identifier"]["text"], "a.b%2Ec");
        drop(watch);

        let deleted = client
            .delete(format!("{address}/api/v1/admin/notification/{name}@1"))
            .send()
            .await
            .unwrap();
        assert_eq!(
            deleted.status(),
            StatusCode::OK,
            "{}",
            deleted.text().await.unwrap()
        );
        assert_eq!(stream.info().await.unwrap().state.messages, 3);
        let wiped = client
            .delete(format!("{address}/api/v1/admin/wipe/stream"))
            .json(&json!({"stream_name": event_type}))
            .send()
            .await
            .unwrap();
        assert_eq!(wiped.status(), StatusCode::OK);
        assert_eq!(stream.info().await.unwrap().state.messages, 0);
        js.delete_stream(name).await.unwrap();
    }
    for base in [format!("Generic_{suffix}"), format!("No-schema_{suffix}")] {
        let response = client
            .post(format!("{address}/api/v1/notification"))
            .json(&json!({"event_type": base, "identifier": {"number": 1.45, "text": "a.b%2Ec"}}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let mut stream = js.get_stream(base.to_ascii_uppercase()).await.unwrap();
        let topic = format!("{base}.1%2E45.a%2Eb%252Ec");
        assert_eq!(
            stream.get_raw_message(1).await.unwrap().subject.as_str(),
            topic
        );
        // Generic events lack schema key order for CloudEvent reconstruction;
        // check their replay/watch routing at the backend boundary instead.
        let replay = backend
            .get_messages_batch(BatchParams::new(topic.clone(), 10).with_sequence(1))
            .await
            .unwrap();
        assert_eq!(replay.messages.len(), 1);
        assert_eq!(replay.messages[0].topic, topic);
        let mut watch = backend.subscribe_to_topic(&topic).await.unwrap();
        backend.put_messages(&topic, "{}".into()).await.unwrap();
        let live = tokio::time::timeout(Duration::from_secs(10), watch.next())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(live.topic, topic);
        drop(watch);
        let wiped = client
            .delete(format!("{address}/api/v1/admin/wipe/stream"))
            .json(&json!({"stream_name": base.to_ascii_uppercase()}))
            .send()
            .await
            .unwrap();
        assert_eq!(wiped.status(), StatusCode::OK);
        assert_eq!(stream.info().await.unwrap().state.messages, 0);
        js.delete_stream(base.to_ascii_uppercase()).await.unwrap();
    }
    backend.shutdown().await.unwrap();
    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(15), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
