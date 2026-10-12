// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! `collect_replay`: the bounded, non-streaming replay over the in-memory
//! backend. Validation, filtering, bounds, limits, and the deadline.

use anyhow::Result;
use aviso_server::auth::User;
use aviso_server::auth::stream_access::StreamAccessError;
use aviso_server::configuration::Settings;
use aviso_server::notification_backend::{
    BackendCapabilities, DeleteMessageResult, NotificationBackend, Subscription, WipeStreamResult,
    build_backend, replay::BatchParams,
};
use aviso_server::sse::collect::{
    CollectLimits, CollectReplayError, CollectedReplay, ReplayAccess, collect_replay,
};
use aviso_server::types::BatchResult;
use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

const BASE: &str = "CollectReplay";
/// Replay limit of the `collect` schema.
const CAP: usize = 8;

fn init_config() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        let identifier = json!({
            "class": {"type": "StringHandler", "required": true},
            "number": {"type": "IntHandler", "required": true}
        });
        let mut config: Settings = serde_json::from_value(json!({
            "application": {"host": "127.0.0.1", "port": 0, "base_url": "http://localhost"},
            "notification_backend": {"kind": "in_memory"},
            "notification_schema": {
                "collect": {
                    "topic": {"base": BASE, "key_order": ["class", "number"]},
                    "identifier": identifier,
                    "payload": {"required": false},
                    "max_historical_notifications": CAP
                },
                "guarded": {
                    "topic": {"base": "CollectGuarded", "key_order": ["class", "number"]},
                    "identifier": identifier,
                    "payload": {"required": false},
                    "auth": {"required": true, "read_roles": {"ecmwf": ["reader"]}}
                },
                "plugged": {
                    "topic": {"base": "CollectPlugged", "key_order": ["class", "number"]},
                    "identifier": identifier,
                    "payload": {"required": false},
                    "auth": {"required": true, "plugins": ["ecpds"]}
                }
            }
        }))
        .unwrap();
        config.watch_endpoint.replay_batch_delay_ms = 0;
        config.watch_endpoint.replay_batch_size = 3;
        config.init_global_config();
    });
}

async fn backend_with_history() -> (Arc<dyn NotificationBackend>, Vec<DateTime<Utc>>) {
    init_config();
    let backend = build_backend(&serde_json::from_value(json!({"kind": "in_memory"})).unwrap())
        .await
        .unwrap();
    for number in 1..=10u64 {
        let class = if number % 2 == 0 { "od" } else { "rd" };
        backend
            .put_messages(
                &format!("{BASE}.{class}.{number}"),
                json!({"number": number}).to_string(),
            )
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    let batch = backend
        .get_messages_batch(BatchParams::new(format!("{BASE}.*.*"), 100).with_sequence(0))
        .await
        .unwrap();
    let times = batch
        .messages
        .iter()
        .map(|m| m.timestamp.unwrap())
        .collect();
    (backend, times)
}

fn open_access() -> ReplayAccess<'static> {
    ReplayAccess {
        auth_enabled: false,
        user: None,
        admin_roles: None,
    }
}

fn limits(max_results: usize) -> CollectLimits {
    CollectLimits {
        max_results: NonZeroUsize::new(max_results).unwrap(),
        timeout: Duration::from_secs(10),
    }
}

async fn collect(
    backend: &Arc<dyn NotificationBackend>,
    request: Value,
    max_results: usize,
) -> Result<CollectedReplay, CollectReplayError> {
    let body = serde_json::to_vec(&request).unwrap();
    collect_replay(
        &body,
        backend.clone(),
        open_access(),
        limits(max_results),
        "req-collect",
    )
    .await
}

fn numbers(collected: &CollectedReplay) -> Vec<u64> {
    collected
        .events
        .iter()
        .map(|event| event["data"]["payload"]["number"].as_u64().unwrap())
        .collect()
}

fn request(class: &str, extra: Value) -> Value {
    let mut request = json!({
        "event_type": "collect",
        "identifier": {"class": class, "number": {"gte": 0}},
    });
    for (key, value) in extra.as_object().unwrap() {
        request[key] = value.clone();
    }
    request
}

#[tokio::test]
async fn filters_apply_to_collected_events() {
    let (backend, _) = backend_with_history().await;
    let collected = collect(&backend, request("od", json!({"from_id": "1"})), 50)
        .await
        .unwrap();
    assert_eq!(numbers(&collected), vec![2, 4, 6, 8, 10]);
    assert!(!collected.truncated);
    assert_eq!(collected.event_type, "collect");
    assert_eq!(collected.events[0]["specversion"], "1.0");
}

#[tokio::test]
async fn max_results_truncates_and_flags() {
    let (backend, _) = backend_with_history().await;
    let collected = collect(&backend, request("rd", json!({"from_id": "1"})), 2)
        .await
        .unwrap();
    assert_eq!(numbers(&collected), vec![1, 3]);
    assert!(collected.truncated);

    let exact = collect(&backend, request("rd", json!({"from_id": "1"})), 5)
        .await
        .unwrap();
    assert_eq!(numbers(&exact), vec![1, 3, 5, 7, 9]);
    assert!(!exact.truncated, "a full quota alone is not truncation");
}

#[tokio::test]
async fn schema_replay_limit_still_applies_below_max_results() {
    let (backend, _) = backend_with_history().await;
    let all = json!({
        "event_type": "collect",
        "identifier": {"class": "*", "number": {"gte": 0}},
        "from_id": "1"
    });
    let collected = collect(&backend, all, 100).await.unwrap();
    assert_eq!(numbers(&collected), (1..=8).collect::<Vec<u64>>());
    assert!(collected.truncated);
}

#[tokio::test]
async fn date_and_sequence_bounds_are_honoured() {
    let (backend, times) = backend_with_history().await;
    let from = times[2].to_rfc3339_opts(SecondsFormat::Nanos, true);
    let to = times[7].to_rfc3339_opts(SecondsFormat::Nanos, true);
    let collected = collect(
        &backend,
        request("od", json!({"from_date": from, "to_date": to})),
        50,
    )
    .await
    .unwrap();
    assert_eq!(numbers(&collected), vec![4, 6, 8]);

    let collected = collect(
        &backend,
        request("rd", json!({"from_id": "3", "to_id": "7"})),
        50,
    )
    .await
    .unwrap();
    assert_eq!(numbers(&collected), vec![3, 5, 7]);
}

#[tokio::test]
async fn invalid_requests_are_typed_errors() {
    let (backend, _) = backend_with_history().await;
    let unknown = collect(
        &backend,
        json!({"event_type": "nope", "identifier": {}, "from_id": "1"}),
        5,
    )
    .await;
    match unknown {
        Err(CollectReplayError::UnknownEventType {
            event_type,
            configured,
        }) => {
            assert_eq!(event_type, "nope");
            assert_eq!(configured, vec!["collect", "guarded", "plugged"]);
        }
        other => panic!("expected unknown event type, got {other:?}"),
    }

    let missing_start = collect(&backend, request("od", json!({})), 5).await;
    assert!(matches!(
        missing_start,
        Err(CollectReplayError::Validation(_))
    ));

    let body = b"{not json";
    let parse = collect_replay(
        body,
        backend.clone(),
        open_access(),
        limits(5),
        "req-collect",
    )
    .await;
    assert!(matches!(parse, Err(CollectReplayError::Parse(_))));
}

#[tokio::test]
async fn stream_auth_is_enforced() {
    let (backend, _) = backend_with_history().await;
    let body = serde_json::to_vec(&json!({
        "event_type": "guarded",
        "identifier": {"class": "od", "number": {"gte": 0}},
        "from_id": "1"
    }))
    .unwrap();
    let admin_roles = HashMap::from([("ecmwf".to_string(), vec!["admin".to_string()])]);
    let outsider = User {
        username: "mcp-user".to_string(),
        realm: Some("mcp".to_string()),
        roles: vec!["reader".to_string()],
        attributes: HashMap::new(),
    };
    let reader = User {
        realm: Some("ecmwf".to_string()),
        ..outsider.clone()
    };
    let mut outcomes = Vec::new();
    for user in [None, Some(&outsider), Some(&reader)] {
        let access = ReplayAccess {
            auth_enabled: true,
            user,
            admin_roles: Some(&admin_roles),
        };
        outcomes.push(collect_replay(&body, backend.clone(), access, limits(5), "req").await);
    }
    assert!(matches!(
        outcomes[0],
        Err(CollectReplayError::Access(
            StreamAccessError::Unauthenticated
        ))
    ));
    assert!(matches!(
        outcomes[1],
        Err(CollectReplayError::Access(
            StreamAccessError::MissingReadRole
        ))
    ));
    assert!(outcomes[2].is_ok());

    let plugged = serde_json::to_vec(&json!({
        "event_type": "plugged",
        "identifier": {"class": "od", "number": {"gte": 0}},
        "from_id": "1"
    }))
    .unwrap();
    let access = ReplayAccess {
        auth_enabled: true,
        user: Some(&reader),
        admin_roles: Some(&admin_roles),
    };
    let refused = collect_replay(&plugged, backend.clone(), access, limits(5), "req").await;
    assert!(matches!(
        refused,
        Err(CollectReplayError::UnsupportedAuthPlugin { .. })
    ));
}

/// Backend whose replay boundary lookup never finishes, or fails.
struct StuckBackend {
    fail: bool,
}

#[async_trait::async_trait]
impl NotificationBackend for StuckBackend {
    fn capabilities(&self) -> BackendCapabilities {
        aviso_server::notification_backend::IN_MEMORY_CAPABILITIES
    }
    async fn history_end(&self, _: &str) -> Result<u64> {
        if self.fail {
            anyhow::bail!("injected boundary failure");
        }
        std::future::pending().await
    }
    async fn first_sequence_after(&self, _: &str, _: DateTime<Utc>) -> Result<Option<u64>> {
        std::future::pending().await
    }
    async fn get_messages_batch(&self, _: BatchParams) -> Result<BatchResult> {
        std::future::pending().await
    }
    async fn subscribe_to_topic(&self, _: &str) -> Result<Subscription> {
        anyhow::bail!("collect must not subscribe")
    }
    async fn put_messages(&self, _: &str, _: String) -> Result<()> {
        anyhow::bail!("collect must not publish")
    }
    async fn put_message_with_headers(
        &self,
        _: &str,
        _: Option<HashMap<String, String>>,
        _: String,
    ) -> Result<()> {
        anyhow::bail!("collect must not publish")
    }
    async fn wipe_stream(&self, _: &str) -> Result<WipeStreamResult> {
        anyhow::bail!("collect must not wipe")
    }
    async fn wipe_all(&self) -> Result<()> {
        anyhow::bail!("collect must not wipe")
    }
    async fn delete_message(&self, _: &str, _: u64) -> Result<DeleteMessageResult> {
        anyhow::bail!("collect must not delete")
    }
}

#[tokio::test]
async fn timeout_and_backend_failure_are_typed_errors() {
    init_config();
    let body = serde_json::to_vec(&request("od", json!({"from_id": "1"}))).unwrap();
    let stuck: Arc<dyn NotificationBackend> = Arc::new(StuckBackend { fail: false });
    let deadline = CollectLimits {
        max_results: NonZeroUsize::new(5).unwrap(),
        timeout: Duration::from_millis(50),
    };
    let timed_out = collect_replay(&body, stuck, open_access(), deadline, "req").await;
    assert!(matches!(timed_out, Err(CollectReplayError::Timeout(_))));

    let failing: Arc<dyn NotificationBackend> = Arc::new(StuckBackend { fail: true });
    let failed = collect_replay(&body, failing, open_access(), limits(5), "req").await;
    assert!(matches!(failed, Err(CollectReplayError::Backend(_))));
}
