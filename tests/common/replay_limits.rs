// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use aviso_server::configuration::Settings;
use aviso_server::startup::Application;
use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use std::sync::OnceLock;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

use super::{BATCH_SIZE, CAP};

#[path = "empty_replay.rs"]
mod empty_replay;

fn settings() -> Settings {
    static SETTINGS: OnceLock<Settings> = OnceLock::new();
    SETTINGS
        .get_or_init(|| {
            let mut settings: Settings = serde_json::from_value(json!({
                "application": {"host": "127.0.0.1", "port": 0,
                    "base_url": "http://localhost", "static_files_path": "./src/static"},
                "notification_backend": {"kind": "in_memory",
                    "in_memory": {"max_history_per_topic": 100}},
                "watch_endpoint": {"replay_batch_size": BATCH_SIZE,
                    "sse_heartbeat_interval_sec": 30, "connection_max_duration_sec": 60,
                    "concurrent_notification_processing": 1,
                    "max_historical_notifications": CAP, "replay_batch_delay_ms": 0},
                "notification_schema": {"quota": {
                    "topic": {"base": format!("Quota_{}", uuid::Uuid::new_v4().simple()),
                        "key_order": ["group", "number"]},
                    "identifier": {
                        "group": {"type": "StringHandler", "required": true},
                        "number": {"type": "FloatHandler", "required": true},
                        "polygon": {"type": "PolygonHandler", "required": false}
                    }
                }}
            }))
            .unwrap();
            let schemas = settings.notification_schema.as_mut().unwrap();
            for (name, cap) in [("lower", 1), ("higher", 5)] {
                let mut schema = schemas["quota"].clone();
                schema.topic.as_mut().unwrap().base =
                    format!("{name}_{}", uuid::Uuid::new_v4().simple());
                schema.max_historical_notifications = std::num::NonZeroUsize::new(cap);
                schemas.insert(name.into(), schema);
            }
            settings.init_global_config();
            settings
        })
        .clone()
}

fn polygon(offset: usize) -> Value {
    json!([
        [offset, 0],
        [offset + 1, 0],
        [offset + 1, 1],
        [offset, 1],
        [offset, 0]
    ])
}

async fn publish(client: &Client, address: &str, group: &str, number: usize, offset: usize) {
    let response = client
        .post(format!("{address}/api/v1/notification"))
        .json(&json!({"event_type": "quota", "identifier": {
            "group": group, "number": number, "polygon": polygon(offset)
        }, "payload": {"number": number}}))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let body = response.text().await.unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
}

async fn replay(client: &Client, address: &str, endpoint: &str, identifier: Value) -> String {
    let response = client
        .post(format!("{address}/api/v1/{endpoint}"))
        .json(&json!({"event_type": "quota", "identifier": identifier, "from_id": "0"}))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let body = response
        .text()
        .await
        .expect("replay or truncated watch must close promptly");
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

fn assert_replay(body: &str, expected: &[usize], truncated: bool) {
    assert_replay_with_cap(body, expected, truncated, CAP);
}

fn assert_replay_with_cap(body: &str, expected: &[usize], truncated: bool, cap: usize) {
    let events: Vec<Value> = body
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(|data| serde_json::from_str(data.trim()).unwrap())
        .collect();
    assert_eq!(events[0]["type"], "replay_started", "{body}");
    assert_eq!(events[0]["batch_size"], BATCH_SIZE);
    assert!(events[0]["request_id"].is_string());
    let numbers: Vec<usize> = events
        .iter()
        .filter(|event| event.get("specversion").is_some())
        .map(|event| usize::try_from(event["data"]["payload"]["number"].as_u64().unwrap()).unwrap())
        .collect();
    assert_eq!(numbers, expected, "{body}");
    assert!(!body.contains("event: live-notification"), "{body}");
    assert!(!body.contains("event: error"), "{body}");
    let limits: Vec<_> = events
        .iter()
        .filter(|event| event["type"] == "notification_replay_limit_reached")
        .collect();
    assert_eq!(limits.len(), usize::from(truncated), "{body}");
    if truncated {
        assert_eq!(limits[0]["max_allowed"], cap);
        assert!(limits[0]["topic"].is_string());
        assert!(limits[0]["timestamp"].is_string());
        assert_eq!(
            events[events.len() - 2]["type"],
            "notification_replay_limit_reached"
        );
    }
    assert_eq!(
        events
            .iter()
            .filter(|event| event["type"] == "replay_completed")
            .count(),
        usize::from(!truncated),
        "{body}"
    );
    assert_eq!(events.last().unwrap()["reason"], "end_of_stream");
}

async fn exercise(jetstream: bool) {
    let mut config = settings();
    if jetstream {
        config.notification_backend = serde_json::from_value(json!({
            "kind": "jetstream", "jetstream": {
                "nats_url": std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into())
            }
        })).unwrap();
    }
    let shutdown = CancellationToken::new();
    let app = Application::build(config.clone(), shutdown.clone())
        .await
        .unwrap();
    let address = format!("http://127.0.0.1:{}", app.port());
    let server = tokio::spawn(app.run_until_stopped());
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap();

    // Different schema caps coexist with the inherited global cap in one process.
    for (event_type, cap) in [("lower", 1), ("higher", 5), ("quota", CAP)] {
        for number in 0..6 {
            let response = client
                .post(format!("{address}/api/v1/notification"))
                .json(&json!({"event_type": event_type, "identifier": {
                    "group": "override", "number": number, "polygon": polygon(0)
                }, "payload": {"number": number}}))
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
        for endpoint in ["replay", "watch", "replay"] {
            let response = client
                .post(format!("{address}/api/v1/{endpoint}"))
                .json(&json!({"event_type": event_type, "identifier": {
                    "group": "override", "number": {"gte": 0}
                }, "from_id": "0"}))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body = response.text().await.unwrap();
            assert_replay_with_cap(&body, &(0..cap).collect::<Vec<_>>(), true, cap);
        }
    }

    // Separate groups let all cases share one immutable global configuration.
    for count in 0..=4 {
        let group = format!("count{count}");
        for number in 0..count {
            publish(&client, &address, &group, number, 0).await;
        }
        let expected: Vec<_> = (0..count).take(CAP).collect();
        let identifier = json!({"group": group, "number": {"gte": 0}});
        for _ in 0..2 {
            let body = replay(&client, &address, "replay", identifier.clone()).await;
            assert_replay(&body, &expected, count > CAP);
        }
        if count > CAP {
            let body = replay(&client, &address, "watch", identifier).await;
            assert_replay(&body, &expected, true);
        }
    }

    for number in 0..8 {
        // Numeric exclusions precede the spatial exclusions, then two matches,
        // then excluded history after the exact quota boundary.
        let offset = if [4, 5].contains(&number) { 0 } else { 20 };
        publish(&client, &address, "filtered", number, offset).await;
    }
    for (constraint, mut expected) in [(json!({"gte": 2}), vec![4, 5]), (json!({"gte": 9}), vec![])]
    {
        let body = replay(
            &client,
            &address,
            "replay",
            json!({
                "group": "filtered", "number": constraint, "polygon": polygon(0)
            }),
        )
        .await;
        let truncated = expected.len() > CAP;
        expected.truncate(CAP);
        assert_replay(&body, &expected, truncated);
    }
    let body = replay(
        &client,
        &address,
        "replay",
        json!({
            "group": "filtered", "number": {"gte": 2}
        }),
    )
    .await;
    let expected: Vec<_> = (2..2 + CAP).collect();
    assert_replay(&body, &expected, true);

    // An exhausted, fully filtered watch is allowed to enter live mode.
    // Shutdown must still interrupt it after the normal completion control.
    let mut response = client
        .post(format!("{address}/api/v1/watch"))
        .json(&json!({"event_type": "quota", "identifier": {
            "group": "filtered", "number": {"gte": 9}
        }, "from_id": "0"}))
        .send()
        .await
        .unwrap();
    let mut body = String::new();
    while !body.contains("replay_completed") {
        body.push_str(std::str::from_utf8(&response.chunk().await.unwrap().unwrap()).unwrap());
    }
    for number in 9..12 {
        publish(&client, &address, "filtered", number, 0).await;
    }
    while body.matches("event: live-notification").count() < 3 {
        body.push_str(std::str::from_utf8(&response.chunk().await.unwrap().unwrap()).unwrap());
    }
    assert!(
        !body.contains("notification_replay_limit_reached"),
        "{body}"
    );
    shutdown.cancel();
    while let Some(chunk) = response.chunk().await.unwrap() {
        body.push_str(std::str::from_utf8(&chunk).unwrap());
    }
    assert!(body.contains("server_shutdown"), "{body}");
    server.await.unwrap().unwrap();
    rendering_failures_do_not_consume_quota(&config).await;
    if jetstream {
        let nats = async_nats::connect(
            std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into()),
        )
        .await
        .unwrap();
        let jetstream = async_nats::jetstream::new(nats);
        for schema in config.notification_schema.as_ref().unwrap().values() {
            jetstream
                .delete_stream(schema.topic.as_ref().unwrap().base.to_uppercase())
                .await
                .unwrap();
        }
    }
}

async fn rendering_failures_do_not_consume_quota(config: &Settings) {
    use actix_web::{App, HttpServer, web};
    use aviso_server::notification_backend::build_backend;
    use std::collections::HashMap;

    let backend = build_backend(&config.notification_backend).await.unwrap();
    let base = &config.notification_schema.as_ref().unwrap()["quota"]
        .topic
        .as_ref()
        .unwrap()
        .base;
    // Malformed stored geometry passes identifier filtering but fails rendering.
    // Place failures before, between, and after the successfully rendered quota.
    for count in [0, CAP, CAP + 1] {
        for number in 0..=count {
            backend
                .put_message_with_headers(
                    &format!("{base}.render{count}.{}", number * 2),
                    Some(HashMap::from([(
                        "spatial_geometry".into(),
                        "invalid".into(),
                    )])),
                    json!({"number": number}).to_string(),
                )
                .await
                .unwrap();
            if number < count {
                backend
                    .put_messages(
                        &format!("{base}.render{count}.{}", number * 2 + 1),
                        json!({"number": number}).to_string(),
                    )
                    .await
                    .unwrap();
            }
        }
    }
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let config = config.clone();
    let server = HttpServer::new(move || {
        App::new()
            .wrap(tracing_actix_web::TracingLogger::default())
            .app_data(web::Data::new(config.clone()))
            .app_data(web::Data::new(backend.clone()))
            .app_data(web::Data::new(CancellationToken::new()))
            .route(
                "/api/v1/replay",
                web::post().to(aviso_server::routes::replay::replay),
            )
            .route(
                "/api/v1/watch",
                web::post().to(aviso_server::routes::watch::watch),
            )
    })
    .workers(1)
    .listen(listener)
    .unwrap()
    .run();
    let handle = server.handle();
    let task = tokio::spawn(server);
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap();
    for count in [0, CAP, CAP + 1] {
        for endpoint in if count > CAP {
            vec!["replay", "watch"]
        } else {
            vec!["replay"]
        } {
            let body = replay(
                &client,
                &address,
                endpoint,
                json!({"group": format!("render{count}"), "number": {"gte": 0}}),
            )
            .await;
            assert_eq!(
                body.matches("event: error").count(),
                count.min(CAP) + 1,
                "{body}"
            );
            let without_errors = body
                .split("\n\n")
                .filter(|frame| !frame.starts_with("event: error"))
                .collect::<Vec<_>>()
                .join("\n\n");
            assert_replay(
                &without_errors,
                &(0..count.min(CAP)).collect::<Vec<_>>(),
                count > CAP,
            );
        }
    }
    handle.stop(true).await;
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn in_memory_replay_quota() {
    exercise(false).await;
}

#[tokio::test]
async fn jetstream_replay_quota() {
    if std::env::var("AVISO_RUN_NATS_TESTS")
        .is_ok_and(|value| value == "1" || value.eq_ignore_ascii_case("true"))
    {
        exercise(true).await;
    }
}

#[actix_web::test]
async fn cancellation_interrupts_empty_progress_batches() {
    use actix_web::{App, test, web};
    use aviso_server::notification_backend::NotificationBackend;
    use std::sync::{Arc, atomic::Ordering};

    let config = settings();
    let shutdown = CancellationToken::new();
    let backend = Arc::new(empty_replay::EmptyReplay {
        calls: Default::default(),
        shutdown: shutdown.clone(),
    });
    let service = test::init_service(
        App::new()
            .wrap(tracing_actix_web::TracingLogger::default())
            .app_data(web::Data::new(config))
            .app_data(web::Data::new(
                backend.clone() as Arc<dyn NotificationBackend>
            ))
            .app_data(web::Data::new(shutdown))
            .route(
                "/replay",
                web::post().to(aviso_server::routes::replay::replay),
            ),
    )
    .await;
    let request = test::TestRequest::post()
        .uri("/replay")
        .set_json(json!({"event_type": "quota", "identifier": {
            "group": "empty", "number": {"gte": 0}
        }, "from_id": "0"}))
        .to_request();
    let response = test::call_service(&service, request).await;
    assert_eq!(response.status(), actix_web::http::StatusCode::OK);
    let bytes = test::read_body(response).await;
    let body = std::str::from_utf8(&bytes).unwrap();
    assert_eq!(backend.calls.load(Ordering::SeqCst), 2, "{body}");
    assert!(body.contains("server_shutdown"), "{body}");
    assert!(!body.contains("replay_completed"), "{body}");
    assert!(
        !body.contains("notification_replay_limit_reached"),
        "{body}"
    );
}
