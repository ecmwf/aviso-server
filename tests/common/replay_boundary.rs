use actix_web::{App, HttpServer, web};
use anyhow::Result;
use aviso_server::configuration::Settings;
use aviso_server::notification_backend::{
    BackendCapabilities, DeleteMessageResult, NotificationBackend, Subscription, WipeStreamResult,
    build_backend, replay::BatchParams,
};
use aviso_server::types::BatchResult;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

// Inject publications after the real backend captures H, before HTTP setup
// returns, and before every batch. No sleeps or response-buffer timing races.
struct PublishingBackend {
    inner: Arc<dyn NotificationBackend>,
    topic: String,
    batches: AtomicUsize,
    fail_setup: bool,
}

#[async_trait::async_trait]
impl NotificationBackend for PublishingBackend {
    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }
    async fn subscribe_to_topic(&self, topic: &str) -> Result<Subscription> {
        if self.fail_setup {
            anyhow::bail!("injected boundary setup failure");
        }
        let subscription = self.inner.subscribe_to_topic(topic).await?;
        self.inner
            .put_messages(&self.topic, json!({"number": 100}).to_string())
            .await?;
        Ok(subscription)
    }
    async fn history_end(&self, topic: &str) -> Result<u64> {
        if self.fail_setup {
            anyhow::bail!("injected boundary setup failure");
        }
        let h = self.inner.history_end(topic).await?;
        self.inner
            .put_messages(&self.topic, json!({"number": 100}).to_string())
            .await?;
        Ok(h)
    }
    async fn get_messages_batch(&self, params: BatchParams) -> Result<BatchResult> {
        assert_ne!(params.end_sequence, u64::MAX);
        let n = self.batches.fetch_add(1, Ordering::SeqCst);
        assert!(n < 30, "moving tail must not keep replay alive");
        self.inner
            .put_messages(&self.topic, json!({"number": 101+n}).to_string())
            .await?;
        self.inner
            .put_messages(&format!("{}.excluded", self.topic), "{}".into())
            .await?;
        self.inner.get_messages_batch(params).await
    }
    async fn put_messages(&self, topic: &str, payload: String) -> Result<()> {
        self.inner.put_messages(topic, payload).await
    }
    async fn put_message_with_headers(
        &self,
        topic: &str,
        headers: Option<HashMap<String, String>>,
        payload: String,
    ) -> Result<()> {
        self.inner
            .put_message_with_headers(topic, headers, payload)
            .await
    }
    async fn wipe_stream(&self, key: &str) -> Result<WipeStreamResult> {
        self.inner.wipe_stream(key).await
    }
    async fn wipe_all(&self) -> Result<()> {
        self.inner.wipe_all().await
    }
    async fn delete_message(&self, key: &str, seq: u64) -> Result<DeleteMessageResult> {
        self.inner.delete_message(key, seq).await
    }
}

fn settings() -> Settings {
    static SETTINGS: OnceLock<Settings> = OnceLock::new();
    SETTINGS
        .get_or_init(|| {
            let settings: Settings = serde_json::from_value(json!({
                "application": {"host": "127.0.0.1", "port": 0, "base_url": "http://localhost"},
                "notification_backend": {"kind": "in_memory"},
                "watch_endpoint": {"replay_batch_size": super::BATCH_SIZE,
                    "max_historical_notifications": 2, "replay_batch_delay_ms": 0,
                    "sse_heartbeat_interval_sec": 30, "connection_max_duration_sec": 60,
                    "concurrent_notification_processing": 1},
                "notification_schema": {"boundary": {
                    "topic": {"base": format!("HttpBoundary{}", uuid::Uuid::new_v4().simple()),
                        "key_order": ["group", "number"]},
                    "identifier": {"group": {"type": "StringHandler", "required": true},
                    "number": {"type": "IntHandler", "required": true},
                    "polygon": {"type": "PolygonHandler", "required": false}},
                    "storage_policy": {"allow_duplicates": true}
                }}
            }))
            .unwrap();
            settings.init_global_config();
            settings
        })
        .clone()
}

async fn exercise(jetstream: bool) {
    let mut settings = settings();
    if jetstream {
        settings.notification_backend = serde_json::from_value(json!({"kind": "jetstream",
            "jetstream": {"nats_url": std::env::var("NATS_URL").unwrap()}}))
        .unwrap();
    }
    let inner = build_backend(&settings.notification_backend).await.unwrap();
    let base = settings.notification_schema.as_ref().unwrap()["boundary"]
        .topic
        .as_ref()
        .unwrap()
        .base
        .clone();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    for endpoint in ["watch", "replay"] {
        for case in [
            "exact",
            "filtered",
            "render",
            "deleted",
            "empty",
            "future_seq",
            "future_date",
            "setup_error",
            "spatial",
        ] {
            if endpoint == "watch" && case == "spatial" {
                continue;
            }
            let group = format!("{endpoint}_{case}");
            let topic = format!("{base}.{group}");
            if case != "empty" {
                let numbers: &[u64] = if case == "exact" { &[1, 2] } else { &[1, 2, 9] };
                for &number in numbers {
                    let mut headers = (case == "render" && number == 9)
                        .then(|| HashMap::from([("spatial_geometry".into(), "invalid".into())]));
                    if case == "spatial" {
                        let geometry = if number == 9 {
                            "(20,0,21,0,21,1,20,1,20,0)"
                        } else {
                            "(0,0,1,0,1,1,0,1,0,0)"
                        };
                        headers = Some(HashMap::from([(
                            "spatial_geometry".into(),
                            geometry.into(),
                        )]));
                    }
                    inner
                        .put_message_with_headers(
                            &format!("{topic}.{number}"),
                            headers,
                            json!({"number": number}).to_string(),
                        )
                        .await
                        .unwrap();
                }
                if case == "deleted" {
                    let h = inner.history_end(&topic).await.unwrap();
                    inner.delete_message(&base, h).await.unwrap();
                }
            }
            let backend = Arc::new(PublishingBackend {
                inner: inner.clone(),
                topic: format!("{topic}.3"),
                batches: AtomicUsize::new(0),
                fail_setup: case == "setup_error",
            });
            let app_backend = backend.clone();
            let config = settings.clone();
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = format!("http://{}", listener.local_addr().unwrap());
            let shutdown = CancellationToken::new();
            let app_shutdown = shutdown.clone();
            let server = HttpServer::new(move || {
                App::new()
                    .wrap(tracing_actix_web::TracingLogger::default())
                    .app_data(web::Data::new(config.clone()))
                    .app_data(web::Data::new(
                        app_backend.clone() as Arc<dyn NotificationBackend>
                    ))
                    .app_data(web::Data::new(app_shutdown.clone()))
                    .route("/watch", web::post().to(aviso_server::routes::watch::watch))
                    .route(
                        "/replay",
                        web::post().to(aviso_server::routes::replay::replay),
                    )
            })
            .workers(1)
            .listen(listener)
            .unwrap()
            .run();
            let handle = server.handle();
            let task = tokio::spawn(server);
            let constraint = if case == "render" || case == "spatial" {
                json!({"gte": 0})
            } else if case == "filtered" {
                json!({"between": [3, 4]})
            } else {
                json!({"between": [0, 4]})
            };
            let mut request = json!({"event_type": "boundary", "identifier": {
                "group": group, "number": constraint}, "from_id": "0"});
            if case == "spatial" {
                request["identifier"]["point"] = json!("0.5,0.5");
            }
            if case == "future_seq" {
                request["from_id"] = json!(u64::MAX.to_string());
            }
            if case == "future_date" {
                request.as_object_mut().unwrap().remove("from_id");
                request["from_date"] =
                    json!((chrono::Utc::now() + chrono::Duration::days(1)).to_rfc3339());
            }
            let mut response = client
                .post(format!("{address}/{endpoint}"))
                .json(&request)
                .send()
                .await
                .unwrap();
            if case == "setup_error" {
                assert_eq!(
                    response.status(),
                    reqwest::StatusCode::INTERNAL_SERVER_ERROR
                );
                let body = response.text().await.unwrap();
                assert!(!body.contains("replay_completed"));
                assert_eq!(backend.batches.load(Ordering::SeqCst), 0);
            } else {
                assert_eq!(response.status(), reqwest::StatusCode::OK);
                let mut body = String::new();
                if endpoint == "replay" {
                    body = response.text().await.unwrap();
                } else {
                    while !body.contains("\"number\":100") {
                        body.push_str(
                            std::str::from_utf8(&response.chunk().await.unwrap().unwrap()).unwrap(),
                        );
                    }
                    inner
                        .put_messages(&format!("{topic}.3"), json!({"number": 999}).to_string())
                        .await
                        .unwrap();
                    while !body.contains("\"number\":999") {
                        body.push_str(
                            std::str::from_utf8(&response.chunk().await.unwrap().unwrap()).unwrap(),
                        );
                    }
                    drop(response);
                }
                assert_eq!(
                    body.matches("\"type\":\"replay_started\"").count(),
                    1,
                    "{body}"
                );
                assert_eq!(
                    body.matches("\"type\":\"replay_completed\"").count(),
                    1,
                    "{body}"
                );
                assert!(
                    !body.contains("notification_replay_limit_reached"),
                    "{body}"
                );
                let expected = if ["empty", "filtered", "future_seq", "future_date"].contains(&case)
                {
                    vec![]
                } else {
                    vec![1, 2]
                };
                let replay_numbers: Vec<_> = body
                    .split("\n\n")
                    .filter(|f| f.starts_with("event: replay\n"))
                    .map(|f| {
                        let data = f.lines().find_map(|l| l.strip_prefix("data:")).unwrap();
                        serde_json::from_str::<Value>(data).unwrap()["data"]["payload"]["number"]
                            .as_u64()
                            .unwrap()
                    })
                    .collect();
                assert_eq!(replay_numbers, expected, "{body}");
                if endpoint == "watch" {
                    assert!(
                        body.find("replay_completed").unwrap()
                            < body.find("\"number\":100").unwrap()
                    );
                    assert_eq!(body.matches("\"number\":100").count(), 1, "{body}");
                } else {
                    assert!(!body.contains("event: live-notification"), "{body}");
                }
            }
            shutdown.cancel();
            handle.stop(true).await;
            task.await.unwrap().unwrap();
        }
    }
    inner.wipe_stream(&base).await.unwrap();
    inner.shutdown().await.unwrap();
}

#[tokio::test]
async fn in_memory_http_boundary() {
    exercise(false).await;
}

#[tokio::test]
async fn jetstream_http_boundary() {
    if std::env::var("AVISO_RUN_NATS_TESTS").as_deref() == Ok("1") {
        exercise(true).await;
    }
}
