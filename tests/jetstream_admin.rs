// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use aviso_server::notification::topic_parser::validate_topic_base;
use aviso_server::notification_backend::jetstream::{
    backend::JetStreamBackend, config::JetStreamConfig,
};
use aviso_server::notification_backend::{
    DeleteMessageResult, NotificationBackend, WipeStreamResult,
};
use serde_json::json;

// Destructive: use a dedicated broker/account, never a shared NATS_URL.
// AVISO_RUN_NATS_WIPE_ALL_TESTS=1 NATS_URL=... cargo test --test jetstream_admin
#[tokio::test]
async fn legacy_stream_names_support_delete_wipe_and_wipe_all() {
    if !std::env::var("AVISO_RUN_NATS_WIPE_ALL_TESTS")
        .is_ok_and(|value| value == "1" || value.eq_ignore_ascii_case("true"))
    {
        return;
    }
    let settings = serde_json::from_value(json!({
        "kind": "jetstream",
        "jetstream": {"nats_url": std::env::var("NATS_URL").expect("explicit isolated broker URL")}
    }))
    .unwrap();
    let backend = JetStreamBackend::new(JetStreamConfig::from_backend_settings(&settings).unwrap())
        .await
        .unwrap();
    let suffix = uuid::Uuid::new_v4()
        .simple()
        .to_string()
        .to_ascii_uppercase();
    let mut streams = Vec::new();
    for prefix in ["_WEATHER", "WEATHER%2EV1"] {
        let name = format!("{prefix}_{suffix}");
        assert!(validate_topic_base(&name).is_err());
        let subject = format!("legacy_{prefix}_{suffix}.value");
        let mut stream = backend
            .jetstream
            .create_stream(async_nats::jetstream::stream::Config {
                name: name.clone(),
                subjects: vec![subject.clone()],
                ..Default::default()
            })
            .await
            .unwrap();
        for _ in 0..2 {
            backend
                .jetstream
                .publish(subject.clone(), "{}".into())
                .await
                .unwrap()
                .await
                .unwrap();
        }
        assert_eq!(stream.info().await.unwrap().state.messages, 2);
        assert_eq!(
            backend
                .delete_message(&name.to_ascii_lowercase(), 1)
                .await
                .unwrap(),
            DeleteMessageResult::Deleted
        );
        assert_eq!(stream.info().await.unwrap().state.messages, 1);
        assert_eq!(
            backend
                .wipe_stream(&name.to_ascii_lowercase())
                .await
                .unwrap(),
            WipeStreamResult::Wiped
        );
        assert_eq!(stream.info().await.unwrap().state.messages, 0);
        backend
            .jetstream
            .publish(subject, "{}".into())
            .await
            .unwrap()
            .await
            .unwrap();
        assert_eq!(stream.info().await.unwrap().state.messages, 1);
        streams.push((name, stream));
    }
    backend.wipe_all().await.unwrap();
    for (name, mut stream) in streams {
        assert_eq!(stream.info().await.unwrap().state.messages, 0, "{name}");
        backend.jetstream.delete_stream(name).await.unwrap();
    }
    backend.shutdown().await.unwrap();
}
