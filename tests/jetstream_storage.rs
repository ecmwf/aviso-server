// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use async_nats::jetstream::stream::StorageType;
use aviso_server::configuration::{NotificationBackendSettings, Settings};
use aviso_server::notification_backend::NotificationBackend;
use aviso_server::notification_backend::jetstream::{JetStreamBackend, JetStreamConfig};
use aviso_server::notification_backend::replay::BatchParams;
use futures::StreamExt;
use std::time::Duration;

#[tokio::test]
async fn storage_creation_and_mismatch_preserve_existing_streams() {
    if !std::env::var("AVISO_RUN_NATS_TESTS")
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        return;
    }
    let global: Settings = serde_json::from_value(serde_json::json!({
        "application": { "host": "127.0.0.1", "port": 8000 },
        "notification_backend": { "kind": "jetstream" }
    }))
    .unwrap();
    global.init_global_config();

    for (initial, requested, current_type, requested_type) in [
        (
            Some("file"),
            Some("memory"),
            StorageType::File,
            StorageType::Memory,
        ),
        (
            Some("memory"),
            Some("file"),
            StorageType::Memory,
            StorageType::File,
        ),
        (Some("memory"), None, StorageType::Memory, StorageType::File),
        (None, Some("memory"), StorageType::File, StorageType::Memory),
    ] {
        let settings: NotificationBackendSettings = serde_json::from_value(serde_json::json!({
            "kind": "jetstream",
            "jetstream": {
                "nats_url": std::env::var("NATS_URL").unwrap(),
                "storage_type": initial,
                "max_messages": 10,
                "max_bytes": 10000,
                "retention_time": "1h",
                "max_reconnect_attempts": 1
            }
        }))
        .unwrap();
        let mut backend =
            JetStreamBackend::new(JetStreamConfig::from_backend_settings(&settings).unwrap())
                .await
                .unwrap();
        let base = format!("storage_{}", uuid::Uuid::new_v4().simple());
        let topic = format!("{base}.one");
        let name = backend.ensure_stream_for_topic(&topic).await.unwrap();
        let mut stream = backend.jetstream.get_stream(&name).await.unwrap();
        assert_eq!(stream.cached_info().config.storage, current_type);
        if current_type == StorageType::File {
            let mut omitted = settings.clone();
            omitted.jetstream.as_mut().unwrap().storage_type = None;
            backend.config = JetStreamConfig::from_backend_settings(&omitted).unwrap();
            backend.ensure_stream_for_topic(&topic).await.unwrap();
        }
        // Matching storage must still allow mutable reconciliation.
        backend.config.max_messages = Some(20);
        backend.ensure_stream_for_topic(&topic).await.unwrap();
        assert_eq!(stream.info().await.unwrap().config.max_messages, 20);
        for subject in [&topic, &format!("{base}.two")] {
            backend
                .jetstream
                .publish(subject.to_owned(), "preserved".into())
                .await
                .unwrap()
                .await
                .unwrap();
        }
        let before = stream.info().await.unwrap().clone();

        let mut settings = settings;
        settings.jetstream.as_mut().unwrap().storage_type =
            requested.map(|value| serde_json::from_value(serde_json::json!(value)).unwrap());
        backend.config = JetStreamConfig::from_backend_settings(&settings).unwrap();
        let expected = format!(
            "Stream '{name}' storage mismatch: current {current_type:?}, requested {requested_type:?}"
        );
        // Check both a would-be no-op and simultaneous destructive limit drift.
        for drift in [false, true] {
            backend.config.max_messages = Some(if drift { 1 } else { 20 });
            backend.config.max_bytes = Some(if drift { 1000 } else { 10000 });
            backend.config.retention_time = Some(Duration::from_secs(if drift { 1 } else { 3600 }));
            let errors = [
                backend.ensure_stream_for_topic(&topic).await.unwrap_err(),
                backend
                    .put_messages(&topic, "rejected".to_string())
                    .await
                    .unwrap_err(),
                backend
                    .put_message_with_headers(&topic, None, "rejected".to_string())
                    .await
                    .unwrap_err(),
                backend
                    .get_messages_batch(BatchParams::new(topic.clone(), 10).with_sequence(1))
                    .await
                    .unwrap_err(),
                backend.history_end(&topic).await.unwrap_err(),
                backend
                    .subscribe_to_topic(&topic)
                    .await
                    .err()
                    .expect("watch setup must fail"),
            ];
            for error in errors {
                assert!(format!("{error:#}").contains(&expected), "{error:#}");
            }
        }

        // Hide only the initial lookup to exercise the real broker's create
        // conflict and subsequent reload, without relying on scheduling a race.
        let prefix = format!("test_api_{}", uuid::Uuid::new_v4().simple());
        let mut requests = backend
            .client
            .subscribe(format!("{prefix}.>"))
            .await
            .unwrap();
        backend.client.flush().await.unwrap();
        let client = backend.client.clone();
        let proxy_prefix = prefix.clone();
        let proxy_name = name.clone();
        let proxy = tokio::spawn(async move {
            for (index, operation) in ["INFO", "CREATE", "INFO"].into_iter().enumerate() {
                let message = tokio::time::timeout(Duration::from_secs(10), requests.next())
                    .await
                    .unwrap()
                    .unwrap();
                let suffix = message.subject.strip_prefix(&proxy_prefix).unwrap();
                assert_eq!(suffix, format!(".STREAM.{operation}.{proxy_name}"));
                let payload = if index == 0 {
                    r#"{"type":"io.nats.jetstream.api.v1.stream_info_response","error":{"code":404,"err_code":10059,"description":"stream not found"}}"#.into()
                } else if operation == "INFO" {
                    client
                        .request(format!("$JS.API{suffix}"), message.payload)
                        .await
                        .unwrap()
                        .payload
                } else {
                    let response = client
                        .request(format!("$JS.API{suffix}"), message.payload)
                        .await
                        .unwrap();
                    let body: serde_json::Value =
                        serde_json::from_slice(&response.payload).unwrap();
                    assert_eq!(body["error"]["err_code"], 10058);
                    response.payload
                };
                client
                    .publish(message.reply.unwrap(), payload)
                    .await
                    .unwrap();
            }
        });
        let mut racing_backend = backend.clone();
        racing_backend.jetstream =
            async_nats::jetstream::with_prefix(backend.client.clone(), &prefix);
        let error = racing_backend
            .ensure_stream_for_topic(&topic)
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains(&expected), "{error:#}");
        proxy.await.unwrap();

        let after = stream.info().await.unwrap();
        assert_eq!(after.config, before.config);
        assert_eq!(after.created, before.created);
        assert_eq!(after.state.messages, 2);
        assert_eq!(after.state.first_sequence, before.state.first_sequence);
        assert_eq!(after.state.last_sequence, before.state.last_sequence);
        assert_eq!(after.state.consumer_count, 0);
        for sequence in 1..=2 {
            let message = stream.get_raw_message(sequence).await.unwrap();
            assert_eq!(message.payload.as_ref(), b"preserved");
        }
        backend.jetstream.delete_stream(&name).await.unwrap();
        backend.shutdown().await.unwrap();
    }
}
