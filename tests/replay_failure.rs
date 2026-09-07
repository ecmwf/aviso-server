// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use actix_web::{App, HttpServer, web};
use anyhow::{Result, bail};
use aviso_server::configuration::Settings;
use aviso_server::notification_backend::{
    BackendCapabilities, DeleteMessageResult, IN_MEMORY_CAPABILITIES, NotificationBackend,
    NotificationMessage, WipeStreamResult,
    replay::{BatchParams, StartAt},
};
use aviso_server::types::BatchResult;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio_util::sync::CancellationToken;

struct FailedReplay {
    calls: AtomicUsize,
    live_polls: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl NotificationBackend for FailedReplay {
    fn capabilities(&self) -> BackendCapabilities {
        IN_MEMORY_CAPABILITIES
    }

    async fn get_messages_batch(&self, params: BatchParams) -> Result<BatchResult> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(params.limit, 1);
        let StartAt::Sequence(sequence) = params.start_at else {
            panic!("test uses sequence pagination");
        };
        let sequence = sequence.max(1);
        let successful_batches: u64 = params.topic.split('.').nth(1).unwrap().parse().unwrap();
        if sequence > successful_batches {
            bail!("injected replay fetch failure");
        }
        Ok(BatchResult::new(
            vec![NotificationMessage {
                sequence,
                topic: format!("FailedReplay.{successful_batches}.{sequence}"),
                payload: json!({"number": sequence}).to_string(),
                timestamp: None,
                metadata: None,
            }],
            1,
        ))
    }

    async fn subscribe_to_topic(
        &self,
        _: &str,
    ) -> Result<Box<dyn futures_util::Stream<Item = NotificationMessage> + Unpin + Send>> {
        let polls = self.live_polls.clone();
        Ok(Box::new(futures_util::stream::poll_fn(move |_| {
            polls.fetch_add(1, Ordering::SeqCst);
            panic!("failed catch-up must not poll live delivery");
        })))
    }

    async fn put_messages(&self, _: &str, _: String) -> Result<()> {
        panic!("replay must not publish")
    }
    async fn put_message_with_headers(
        &self,
        _: &str,
        _: Option<HashMap<String, String>>,
        _: String,
    ) -> Result<()> {
        panic!("replay must not publish")
    }
    async fn wipe_stream(&self, _: &str) -> Result<WipeStreamResult> {
        panic!("replay must not wipe storage")
    }
    async fn wipe_all(&self) -> Result<()> {
        panic!("replay must not wipe storage")
    }
    async fn delete_message(&self, _: &str, _: u64) -> Result<DeleteMessageResult> {
        panic!("replay must not delete")
    }
}

#[tokio::test]
async fn failed_catchup_closes_both_http_endpoints_without_completion_or_live() {
    // This binary owns its immutable globals: cap 2, batch size 1.
    let settings: Settings = serde_json::from_value(json!({
        "application": {"host": "127.0.0.1", "port": 0, "base_url": "http://localhost"},
        "notification_backend": {"kind": "in_memory"},
        "watch_endpoint": {
            "max_historical_notifications": 2, "replay_batch_size": 1,
            "replay_batch_delay_ms": 0, "sse_heartbeat_interval_sec": 30,
            "connection_max_duration_sec": 60, "concurrent_notification_processing": 1
        },
        "notification_schema": {"failed_replay": {
            "topic": {"base": "FailedReplay", "key_order": ["stage", "number"]},
            "identifier": {
                "stage": {"type": "StringHandler", "required": true},
                "number": {"type": "IntHandler", "required": true}
            }
        }}
    }))
    .unwrap();
    settings.init_global_config();
    let backend = Arc::new(FailedReplay {
        calls: AtomicUsize::new(0),
        live_polls: Arc::new(AtomicUsize::new(0)),
    });
    let app_backend = backend.clone();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let server = HttpServer::new(move || {
        App::new()
            .wrap(tracing_actix_web::TracingLogger::default())
            .app_data(web::Data::new(settings.clone()))
            .app_data(web::Data::new(
                app_backend.clone() as Arc<dyn NotificationBackend>
            ))
            .app_data(web::Data::new(CancellationToken::new()))
            .route(
                "/replay",
                web::post().to(aviso_server::routes::replay::replay),
            )
            .route("/watch", web::post().to(aviso_server::routes::watch::watch))
    })
    .workers(1)
    .listen(listener)
    .unwrap()
    .run();
    let handle = server.handle();
    let task = tokio::spawn(server);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap();
    for successful_batches in 0..=2 {
        for endpoint in ["replay", "watch"] {
            let before = backend.calls.load(Ordering::SeqCst);
            let response = client
                .post(format!("{address}/{endpoint}"))
                .json(&json!({"event_type": "failed_replay", "identifier": {
                    "stage": successful_batches.to_string(), "number": {"gte": 0}
                }, "from_id": "0"}))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), reqwest::StatusCode::OK);
            let body = response.text().await.unwrap();
            assert_eq!(
                body.matches("event: replay\n").count(),
                successful_batches,
                "{body}"
            );
            assert_eq!(body.matches("event: error\n").count(), 1, "{body}");
            assert!(!body.contains("replay_completed"), "{body}");
            assert!(
                !body.contains("notification_replay_limit_reached"),
                "{body}"
            );
            assert!(!body.contains("event: live-notification"), "{body}");
            let events: Vec<Value> = body
                .lines()
                .filter_map(|line| line.strip_prefix("data:"))
                .map(|data| serde_json::from_str(data.trim()).unwrap())
                .collect();
            assert_eq!(events[0]["type"], "replay_started");
            let error = &events[events.len() - 2];
            assert_eq!(error["error"], "stream_processing_failed");
            assert_eq!(error["message"], "injected replay fetch failure");
            assert_eq!(error["request_id"], events[0]["request_id"]);
            assert_eq!(events.last().unwrap()["reason"], "end_of_stream");
            assert_eq!(
                backend.calls.load(Ordering::SeqCst) - before,
                successful_batches + 1
            );
            assert_eq!(backend.live_polls.load(Ordering::SeqCst), 0);
        }
    }
    handle.stop(true).await;
    task.await.unwrap().unwrap();
}
