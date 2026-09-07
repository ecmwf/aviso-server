// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use aviso_server::configuration::{
    Settings, validate_auth_settings, validate_metrics_settings,
    validate_schema_storage_policy_support, validate_spatial_schema_settings,
    validate_stream_auth_settings, validate_stream_plugin_settings, validate_topic_schema_settings,
};
use aviso_server::notification_backend::jetstream::{config::JetStreamConfig, connection, replay};
use aviso_server::notification_backend::replay::BatchParams;
use futures_util::StreamExt;
use std::time::Duration;

#[tokio::test]
async fn schema_retention_expires_messages_from_replay() {
    if !std::env::var("AVISO_RUN_NATS_TESTS")
        .is_ok_and(|value| value == "1" || value.eq_ignore_ascii_case("true"))
    {
        return;
    }
    let base = format!("Retention_{}", uuid::Uuid::new_v4().simple());
    let settings: Settings = serde_json::from_value(serde_json::json!({
        "application": {
            "host": "127.0.0.1", "port": 0, "base_url": "localhost",
            "static_files_path": "./src/static"
        },
        "notification_backend": {
            "kind": "jetstream",
            "jetstream": {
                "nats_url": std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into()),
                "retention_time": "1d"
            }
        },
        "notification_schema": {
            "expiry": {
                "topic": { "base": base, "key_order": ["id"] },
                "identifier": { "id": { "type": "StringHandler", "required": true } },
                "storage_policy": { "retention_time": "2s" }
            }
        }
    }))
    .unwrap();
    validate_schema_storage_policy_support(&settings).unwrap();
    validate_topic_schema_settings(&settings).unwrap();
    validate_spatial_schema_settings(&settings).unwrap();
    validate_auth_settings(&settings.auth).unwrap();
    validate_stream_plugin_settings(&settings).unwrap();
    validate_stream_auth_settings(&settings).unwrap();
    validate_metrics_settings(&settings).unwrap();
    #[cfg(feature = "ecpds")]
    aviso_server::configuration::validate_ecpds_settings(&settings).unwrap();
    settings.init_global_config();
    let config = JetStreamConfig::from_backend_settings(&settings.notification_backend).unwrap();
    let mut backend = connection::connect(config)
        .await
        .expect("isolated NATS must be reachable");
    let topic = format!("{base}.subject");
    let name = backend.ensure_stream_for_topic(&topic).await.unwrap();
    let mut stream = backend.jetstream.get_stream(&name).await.unwrap();
    assert_eq!(stream.cached_info().config.max_age, Duration::from_secs(2));
    // Observe the actual API: an unchanged ensure must not publish UPDATE.
    let mut updates = backend
        .client
        .subscribe(format!("$JS.API.STREAM.UPDATE.{name}"))
        .await
        .unwrap();
    backend.client.flush().await.unwrap();
    backend.ensure_stream_for_topic(&topic).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(200), updates.next())
            .await
            .is_err()
    );
    updates.unsubscribe().await.unwrap();
    backend
        .jetstream
        .publish(topic.clone(), "{}".into())
        .await
        .unwrap()
        .await
        .unwrap();
    let params = BatchParams::new(topic, 1).with_sequence(1);
    assert_eq!(
        replay::get_messages_batch(&backend, params.clone())
            .await
            .unwrap()
            .messages
            .len(),
        1
    );
    tokio::time::timeout(Duration::from_secs(10), async {
        while stream.info().await.unwrap().state.messages != 0 {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("broker must expire the message");
    assert!(
        replay::get_messages_batch(&backend, params)
            .await
            .unwrap()
            .messages
            .is_empty()
    );
    // Force the lookup/create race deterministically, forwarding all subsequent
    // API requests to the real broker so its conflict and update checks still run.
    let mut stale = stream.info().await.unwrap().config.clone();
    stale.max_age = Duration::from_secs(86400);
    stale.duplicate_window = Duration::from_secs(120);
    backend.jetstream.update_stream(stale).await.unwrap();
    backend
        .jetstream
        .publish(format!("{base}.old"), "{}".into())
        .await
        .unwrap()
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(stream.info().await.unwrap().state.messages, 1);
    let prefix = format!("retention_api_{}", uuid::Uuid::new_v4().simple());
    let mut requests = backend
        .client
        .subscribe(format!("{prefix}.>"))
        .await
        .unwrap();
    backend.client.flush().await.unwrap();
    let client = backend.client.clone();
    backend.jetstream = async_nats::jetstream::with_prefix(backend.client.clone(), &prefix);
    let proxy = tokio::spawn(async move {
        let mut operations = Vec::new();
        for index in 0..4 {
            let request = tokio::time::timeout(Duration::from_secs(10), requests.next())
                .await
                .unwrap()
                .unwrap();
            let operation = request
                .subject
                .as_str()
                .strip_prefix(&format!("{prefix}."))
                .unwrap();
            operations.push(operation.to_string());
            let payload = if index == 0 {
                r#"{"error":{"code":404,"err_code":10059,"description":"stream not found"}}"#.into()
            } else {
                client
                    .request(format!("$JS.API.{operation}"), request.payload)
                    .await
                    .unwrap()
                    .payload
            };
            client
                .publish(request.reply.unwrap(), payload)
                .await
                .unwrap();
        }
        operations
    });
    backend
        .ensure_stream_for_topic(&format!("{base}.subject"))
        .await
        .unwrap();
    let operations = proxy.await.unwrap();
    assert_eq!(
        operations,
        [
            format!("STREAM.INFO.{name}"),
            format!("STREAM.CREATE.{name}"),
            format!("STREAM.INFO.{name}"),
            format!("STREAM.UPDATE.{name}")
        ]
    );
    backend.jetstream = async_nats::jetstream::new(backend.client.clone());
    assert_eq!(
        stream.info().await.unwrap().config.max_age,
        Duration::from_secs(2)
    );
    tokio::time::timeout(Duration::from_secs(10), async {
        while stream.info().await.unwrap().state.messages != 0 {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("shortening must expire previously stored messages");
    backend.jetstream.delete_stream(&name).await.unwrap();
    connection::shutdown(&backend).await.unwrap();
}
