// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! `NotificationBackend::first_sequence_after`, the lookup that turns a replay
//! end date into an end sequence, on both backends.

use aviso_server::configuration::{NotificationBackendSettings, Settings};
use aviso_server::notification_backend::{
    JetStreamBackend, JetStreamConfig, NotificationBackend,
    in_memory::{InMemoryBackend, InMemoryConfig},
    replay::BatchParams,
};
use chrono::{DateTime, TimeDelta, Utc};
use std::sync::Arc;
use std::time::Duration;

#[path = "common/nats.rs"]
mod nats;

use nats::{nats_tests_enabled, nats_url};

/// Stored `(sequence, timestamp)` pairs for `topic`, in sequence order.
async fn stored(backend: &dyn NotificationBackend, topic: &str) -> Vec<(u64, DateTime<Utc>)> {
    let batch = backend
        .get_messages_batch(BatchParams::new(topic.to_string(), 100).with_sequence(0))
        .await
        .unwrap();
    batch
        .messages
        .iter()
        .map(|message| (message.sequence, message.timestamp.unwrap()))
        .collect()
}

/// Checks the contract for one `at`: a message of the topic is below the
/// returned sequence exactly when it was stored at or before `at`. `None`
/// means none was stored after `at`.
fn assert_bounds(messages: &[(u64, DateTime<Utc>)], at: DateTime<Utc>, found: Option<u64>) {
    match found {
        Some(bound) => {
            for (sequence, stored) in messages {
                assert_eq!(
                    *sequence < bound,
                    *stored <= at,
                    "#{sequence} stored at {stored}, bound {bound}, at {at}"
                );
            }
        }
        None => assert!(
            messages.iter().all(|(_, stored)| *stored <= at),
            "a message was stored after {at}, yet none was found"
        ),
    }
}

async fn exercise(backend: Arc<dyn NotificationBackend>, base: String) {
    let topic = format!("{base}.match.yes");
    // A sibling on the same backend subject, and one on another subject.
    let sibling = format!("{base}.match.no");
    let excluded = format!("{base}.excluded.no");

    let now = Utc::now();
    assert_eq!(
        backend.first_sequence_after(&topic, now).await.unwrap(),
        None,
        "nothing is stored yet"
    );

    for subject in [&topic, &sibling, &excluded, &topic, &sibling, &topic] {
        backend.put_messages(subject, "{}".into()).await.unwrap();
    }
    let messages = stored(backend.as_ref(), &topic).await;
    assert_eq!(messages.len(), 3);

    let (first_sequence, first_stored) = messages[0];
    let (_, last_stored) = messages[messages.len() - 1];
    let probes = [
        first_stored - TimeDelta::nanoseconds(1),
        first_stored,
        messages[1].1,
        last_stored,
        last_stored + TimeDelta::days(1),
        // Outside the nanosecond range some backends store times in.
        "1500-01-01T00:00:00Z".parse().unwrap(),
        "2300-01-01T00:00:00Z".parse().unwrap(),
    ];
    for at in probes {
        let found = backend.first_sequence_after(&topic, at).await.unwrap();
        assert_bounds(&messages, at, found);
    }
    assert_eq!(
        backend
            .first_sequence_after(&topic, first_stored - TimeDelta::nanoseconds(1))
            .await
            .unwrap(),
        Some(first_sequence),
        "before everything, the first message bounds the replay"
    );
}

#[tokio::test]
async fn in_memory_first_sequence_after() {
    let backend = InMemoryBackend::new(InMemoryConfig {
        max_history_per_topic: 1000,
        max_topics: 10,
        enable_metrics: false,
    });
    tokio::time::timeout(
        Duration::from_secs(30),
        exercise(
            Arc::new(backend),
            format!("EndPoint{}", uuid::Uuid::new_v4().simple()),
        ),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn jetstream_first_sequence_after() {
    if !nats_tests_enabled() {
        return;
    }
    // The global settings only supply the schema, which tells JetStream the
    // stream's storage policy; the backend itself is built from its own
    // settings below.
    let base = format!("EndPoint{}", uuid::Uuid::new_v4().simple());
    let settings: Settings = serde_json::from_value(serde_json::json!({
        "application": {"host": "127.0.0.1", "port": 0, "base_url": "http://localhost"},
        "notification_backend": {"kind": "in_memory"},
        "notification_schema": {"end_point": {
            "topic": {"base": base, "key_order": ["kind"]},
            "identifier": {"kind": {"type": "StringHandler", "required": true}},
            "storage_policy": {"allow_duplicates": true}
        }}
    }))
    .unwrap();
    settings.init_global_config();
    let settings: NotificationBackendSettings = serde_json::from_value(serde_json::json!({
        "kind": "jetstream", "jetstream": {"nats_url": nats_url()}
    }))
    .unwrap();
    let backend = JetStreamBackend::new(JetStreamConfig::from_backend_settings(&settings).unwrap())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(60), exercise(Arc::new(backend), base))
        .await
        .unwrap();
}
