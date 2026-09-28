// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! `to_id` and `to_date` on `/replay`: the replay stops at the end point, the
//! replay limit counts only what is inside it, and the other endpoints refuse
//! it.

use aviso_server::configuration::Settings;
use aviso_server::notification_backend::{NotificationBackend, build_backend, replay::BatchParams};
use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};
use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use std::sync::Arc;

#[path = "common/nats.rs"]
mod nats;
#[path = "common/replay_app.rs"]
mod replay_app;

use nats::{nats_tests_enabled, nats_url};
use replay_app::{ReplayApp, sse_data_events};

const PUBLISHED: u64 = 10;
/// Replay limit of the schema: a window of up to this many is never cut.
const CAP: u64 = 3;

/// One stored notification: its sequence, storage time and payload number.
struct Stored {
    sequence: u64,
    at: DateTime<Utc>,
    number: u64,
}

async fn stored(backend: &dyn NotificationBackend, base: &str) -> Vec<Stored> {
    let batch = backend
        .get_messages_batch(BatchParams::new(format!("{base}.*"), 100).with_sequence(0))
        .await
        .unwrap();
    batch
        .messages
        .iter()
        .map(|message| {
            let payload: Value = serde_json::from_str(&message.payload).unwrap();
            Stored {
                sequence: message.sequence,
                at: message.timestamp.unwrap(),
                number: payload["number"].as_u64().unwrap(),
            }
        })
        .collect()
}

fn date(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Nanos, true)
}

async fn post(client: &Client, url: String, mut request: Value) -> (StatusCode, String) {
    request["event_type"] = json!("end_point");
    request["identifier"] = json!({"number": {"gte": 0}});
    let response = client.post(url).json(&request).send().await.unwrap();
    let status = response.status();
    (status, response.text().await.unwrap())
}

/// Replays with `cursor` and checks the numbers delivered, whether the replay
/// limit cut it, and the `replay_started` echo of the end.
async fn assert_replay(
    client: &Client,
    address: &str,
    kind: &str,
    cursor: Value,
    expected: &[u64],
    truncated: bool,
    end_sequence: Value,
) {
    let (status, body) = post(client, format!("{address}/replay"), cursor.clone()).await;
    assert_eq!(status, StatusCode::OK, "{kind}: {cursor}: {body}");
    let events = sse_data_events(&body);
    assert_eq!(events[0]["type"], "replay_started", "{kind}: {cursor}");
    assert_eq!(events[0]["end_sequence"], end_sequence, "{kind}: {cursor}");
    let numbers: Vec<u64> = events
        .iter()
        .filter(|event| event.get("specversion").is_some())
        .map(|event| event["data"]["payload"]["number"].as_u64().unwrap())
        .collect();
    assert_eq!(numbers, expected, "{kind}: {cursor}");
    let limited = events
        .iter()
        .any(|event| event["type"] == "notification_replay_limit_reached");
    assert_eq!(limited, truncated, "{kind}: {cursor}");
    assert_eq!(
        body.matches("replay_completed").count(),
        usize::from(!truncated),
        "{kind}: {cursor}"
    );
    assert_eq!(events.last().unwrap()["reason"], "end_of_stream");
}

async fn assert_rejected(client: &Client, url: String, request: Value, code: &str, message: &str) {
    let (status, body) = post(client, url, request.clone()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{request}: {body}");
    let error: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(error["code"], code, "{request}: {body}");
    assert!(
        error["message"].as_str().unwrap().starts_with(message),
        "{request}: {body}"
    );
}

async fn exercise(client: &Client, address: &str, kind: &str, stored: &[Stored]) {
    let seq = |i: usize| stored[i].sequence;
    let numbers = |range: std::ops::RangeInclusive<usize>| -> Vec<u64> {
        stored[range].iter().map(|s| s.number).collect()
    };
    let first = seq(0);

    // to_id: an inclusive end sequence.
    assert_replay(
        client,
        address,
        kind,
        json!({"from_id": first.to_string(), "to_id": seq(1).to_string()}),
        &numbers(0..=1),
        false,
        json!(seq(1)),
    )
    .await;
    assert_replay(
        client,
        address,
        kind,
        json!({"from_id": seq(4).to_string(), "to_id": seq(4).to_string()}),
        &numbers(4..=4),
        false,
        json!(seq(4)),
    )
    .await;
    // An end past the last message ends at the last message.
    assert_replay(
        client,
        address,
        kind,
        json!({"from_id": seq(7).to_string(), "to_id": (seq(9) + 100).to_string()}),
        &numbers(7..=9),
        false,
        json!(seq(9)),
    )
    .await;

    // to_date: an inclusive end time, with either kind of start. The
    // expected notifications come from the stored times, so equal times of
    // neighbouring messages do not matter.
    let window = |from: DateTime<Utc>, to: DateTime<Utc>| -> (Vec<u64>, Value) {
        let inside: Vec<&Stored> = stored
            .iter()
            .filter(|s| s.at >= from && s.at <= to)
            .collect();
        let last = stored
            .iter()
            .filter(|s| s.at <= to)
            .map(|s| s.sequence)
            .max();
        (
            inside.iter().map(|s| s.number).collect(),
            json!(last.unwrap_or(first - 1)),
        )
    };
    let everything = DateTime::<Utc>::MIN_UTC;
    let (expected, end) = window(everything, stored[2].at);
    assert_replay(
        client,
        address,
        kind,
        json!({"from_id": first.to_string(), "to_date": date(stored[2].at)}),
        &expected,
        false,
        end,
    )
    .await;
    let (expected, end) = window(stored[3].at, stored[5].at);
    assert_replay(
        client,
        address,
        kind,
        json!({"from_date": date(stored[3].at), "to_date": date(stored[5].at)}),
        &expected,
        false,
        end,
    )
    .await;
    // An end before every message is an empty replay that completes, even
    // before the range some backends store times in.
    for to_date in [
        date(stored[0].at - TimeDelta::nanoseconds(1)),
        "1500-01-01T00:00:00Z".to_string(),
    ] {
        assert_replay(
            client,
            address,
            kind,
            json!({"from_id": first.to_string(), "to_date": to_date}),
            &[],
            false,
            json!(first - 1),
        )
        .await;
    }
    // An end after every message ends at the last message, even past the
    // range some backends store times in (unix seconds for the year 2286).
    for to_date in ["2100-01-01T00:00:00Z", "10000000000"] {
        assert_replay(
            client,
            address,
            kind,
            json!({"from_id": seq(8).to_string(), "to_date": to_date}),
            &numbers(8..=9),
            false,
            json!(seq(9)),
        )
        .await;
    }

    // The replay limit counts only what is inside the window: a window of
    // exactly CAP notifications is complete, not cut short.
    assert_replay(
        client,
        address,
        kind,
        json!({"from_id": first.to_string(), "to_id": seq(2).to_string()}),
        &numbers(0..=2),
        false,
        json!(seq(2)),
    )
    .await;
    assert_replay(
        client,
        address,
        kind,
        json!({"from_id": first.to_string(), "to_id": seq(3).to_string()}),
        &numbers(0..=2),
        true,
        json!(seq(3)),
    )
    .await;
    // Without an end, the same start is cut by the limit, as before.
    assert_replay(
        client,
        address,
        kind,
        json!({"from_id": first.to_string()}),
        &numbers(0..=2),
        true,
        Value::Null,
    )
    .await;

    let replay = format!("{address}/replay");
    assert_rejected(
        client,
        replay.clone(),
        json!({"from_id": "1", "to_id": "5", "to_date": "2026-01-01T00:00:00Z"}),
        "INVALID_REPLAY_REQUEST",
        "Cannot specify both to_id and to_date",
    )
    .await;
    assert_rejected(
        client,
        replay.clone(),
        json!({"from_id": "5", "to_id": "4"}),
        "INVALID_REPLAY_REQUEST",
        "to_id (4) must not be lower than from_id (5)",
    )
    .await;
    assert_rejected(
        client,
        replay.clone(),
        json!({"from_date": "2026-01-02T00:00:00Z", "to_date": "2026-01-01T00:00:00Z"}),
        "INVALID_REPLAY_REQUEST",
        "to_date (2026-01-01T00:00:00+00:00) must not be earlier",
    )
    .await;
    assert_rejected(
        client,
        replay.clone(),
        json!({"from_id": "1", "to_id": "soon"}),
        "INVALID_REPLAY_REQUEST",
        "to_id must be a valid positive integer",
    )
    .await;
    assert_rejected(
        client,
        replay,
        json!({"to_id": "5"}),
        "INVALID_REPLAY_REQUEST",
        "Replay endpoint requires either from_id or from_date",
    )
    .await;
    assert_rejected(
        client,
        format!("{address}/watch"),
        json!({"to_id": "5"}),
        "INVALID_WATCH_REQUEST",
        "to_id and to_date are only supported for the replay endpoint, not /watch",
    )
    .await;
    assert_rejected(
        client,
        format!("{address}/notification"),
        json!({"to_date": "2026-01-01T00:00:00Z", "payload": {}}),
        "INVALID_NOTIFICATION_REQUEST",
        "to_id and to_date are only supported for the replay endpoint, not /notification",
    )
    .await;
}

#[tokio::test]
async fn replay_stops_at_the_end_point() {
    let base = format!("EndPoint_{}", uuid::Uuid::new_v4().simple());
    let mut config: Settings = serde_json::from_value(json!({
        "application": {"host": "127.0.0.1", "port": 0, "base_url": "http://localhost"},
        "notification_backend": {"kind": "in_memory"},
        "notification_schema": {"end_point": {
            "topic": {"base": base, "key_order": ["number"]},
            "identifier": {"number": {"type": "IntHandler", "required": true}},
            "payload": {"required": false},
            "max_historical_notifications": CAP
        }}
    }))
    .unwrap();
    config.watch_endpoint.replay_batch_delay_ms = 0;
    config.init_global_config();
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .unwrap();
    for kind in ["in_memory", "jetstream"] {
        if kind == "jetstream" && !nats_tests_enabled() {
            continue;
        }
        config.notification_backend =
            serde_json::from_value(json!({"kind": kind, "jetstream": {"nats_url": nats_url()}}))
                .unwrap();
        let backend: Arc<dyn NotificationBackend> =
            build_backend(&config.notification_backend).await.unwrap();
        for number in 1..=PUBLISHED {
            backend
                .put_messages(
                    &format!("{base}.{number}"),
                    json!({"number": number}).to_string(),
                )
                .await
                .unwrap();
        }
        let stored = stored(backend.as_ref(), &base).await;
        assert_eq!(stored.len(), usize::try_from(PUBLISHED).unwrap());

        let app = ReplayApp::start(&config, backend.clone());
        exercise(&client, &app.address, kind, &stored).await;
        app.stop().await;
        backend.wipe_stream(&base.to_uppercase()).await.unwrap();
    }
}
