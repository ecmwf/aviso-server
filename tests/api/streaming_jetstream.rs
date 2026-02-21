use crate::helpers::{spawn_jetstream_test_app, spawn_jetstream_test_app_with_backend_defaults};
use crate::test_utils::{
    post_polygon_notification_for_event_with_identifier, test_polygon, unique_suffix,
};
use async_nats::jetstream::stream::Compression;
use reqwest::StatusCode;
use serde_json::json;
use std::sync::LazyLock;
use tokio::sync::Mutex;
use tokio::time::{Duration, Instant, sleep};

// JetStream-backed integration tests are opt-in:
// AVISO_RUN_NATS_TESTS=1 cargo test --workspace
fn should_run_nats_tests() -> bool {
    std::env::var("AVISO_RUN_NATS_TESTS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

const JETSTREAM_TEST_EVENT_TYPE: &str = "test_polygon_js";
const JETSTREAM_REPLAY_TEST_TIME: &str = "1210";
const JETSTREAM_WATCH_TEST_TIME: &str = "1220";
const JETSTREAM_TEST_DATE: &str = "20250706";
const JETSTREAM_REPLAY_PUBLISH_TEST_TIME: &str = "1310";
const JETSTREAM_POST_REPLAY_PUBLISH_TEST_TIME: &str = "1410";
static JETSTREAM_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

async fn assert_jetstream_test_schema_is_available(client: &reqwest::Client, base_url: &str) {
    let response = client
        .get(format!(
            "{}/api/v1/schema/{}",
            base_url, JETSTREAM_TEST_EVENT_TYPE
        ))
        .send()
        .await
        .expect("failed to query schema endpoint");

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "test schema {JETSTREAM_TEST_EVENT_TYPE} must be available"
    );

    let body: serde_json::Value = response
        .json()
        .await
        .expect("failed to deserialize schema response");

    let returned_event_type = body
        .get("event_type")
        .and_then(|value| value.as_str())
        .expect("schema response missing event_type");
    assert_eq!(
        returned_event_type, JETSTREAM_TEST_EVENT_TYPE,
        "unexpected event_type returned for schema lookup"
    );

    let polygon_rules = body
        .get("schema")
        .and_then(|schema| schema.get("identifier"))
        .and_then(|identifier| identifier.get("polygon"))
        .and_then(|rules| rules.as_array())
        .expect("schema response missing identifier.polygon rules");
    assert!(
        !polygon_rules.is_empty(),
        "schema response must contain polygon identifier rules"
    );
}

async fn assert_status_ok_or_panic(response: reqwest::Response, context: &str) {
    if response.status() != StatusCode::OK {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "<failed to read body>".to_string());
        panic!("{context} failed with status {status}: {body}");
    }
}

async fn fetch_stream_config(stream_name: &str) -> async_nats::jetstream::stream::Config {
    let nats_url =
        std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_string());
    let client = async_nats::connect(nats_url)
        .await
        .expect("failed to connect to NATS for stream inspection");
    let jetstream = async_nats::jetstream::new(client);
    let stream = jetstream
        .get_stream(stream_name)
        .await
        .expect("stream should exist for inspection");
    stream.cached_info().config.clone()
}

#[tokio::test]
async fn jetstream_replay_with_from_date_excludes_older_messages() {
    if !should_run_nats_tests() {
        return;
    }
    let _guard = JETSTREAM_TEST_LOCK.lock().await;

    let app = spawn_jetstream_test_app().await;
    let client = reqwest::Client::new();
    assert_jetstream_test_schema_is_available(&client, &app.address).await;
    let suffix = unique_suffix();

    let old_note = format!("OLD_BEFORE_FROM_DATE_{suffix}");
    let new_note = format!("NEW_AFTER_FROM_DATE_{suffix}");

    let old_response = post_polygon_notification_for_event_with_identifier(
        &client,
        &app.address,
        JETSTREAM_TEST_EVENT_TYPE,
        &old_note,
        test_polygon(),
        JETSTREAM_TEST_DATE,
        JETSTREAM_REPLAY_TEST_TIME,
    )
    .await;
    assert_status_ok_or_panic(old_response, "old notification").await;

    sleep(Duration::from_secs(1)).await;
    let from_date = chrono::Utc::now().to_rfc3339();
    sleep(Duration::from_secs(1)).await;

    let new_response = post_polygon_notification_for_event_with_identifier(
        &client,
        &app.address,
        JETSTREAM_TEST_EVENT_TYPE,
        &new_note,
        test_polygon(),
        JETSTREAM_TEST_DATE,
        JETSTREAM_REPLAY_TEST_TIME,
    )
    .await;
    assert_status_ok_or_panic(new_response, "new notification").await;

    let replay_response = client
        .post(format!("{}/api/v1/replay", &app.address))
        .header("Content-Type", "application/json")
        .json(&json!({
            "event_type": JETSTREAM_TEST_EVENT_TYPE,
            "identifier": {
                "time": JETSTREAM_REPLAY_TEST_TIME,
                "polygon": test_polygon(),
            },
            "from_date": from_date,
        }))
        .send()
        .await
        .expect("failed to call replay endpoint");
    if replay_response.status() != StatusCode::OK {
        let status = replay_response.status();
        let body = replay_response
            .text()
            .await
            .unwrap_or_else(|_| "<failed to read body>".to_string());
        panic!("replay request failed with status {status}: {body}");
    }
    let body = replay_response
        .text()
        .await
        .expect("failed to read replay response body");

    assert!(
        body.contains(&new_note),
        "expected replay to include new message note: {new_note}; body: {body}"
    );
    assert!(
        !body.contains(&old_note),
        "expected replay to exclude old message note: {old_note}; body: {body}"
    );
}

#[tokio::test]
async fn jetstream_watch_without_replay_params_is_live_only() {
    if !should_run_nats_tests() {
        return;
    }
    let _guard = JETSTREAM_TEST_LOCK.lock().await;

    let app = spawn_jetstream_test_app().await;
    let client = reqwest::Client::new();
    assert_jetstream_test_schema_is_available(&client, &app.address).await;
    let suffix = unique_suffix();

    let historical_note = format!("HISTORICAL_BEFORE_WATCH_{suffix}");
    let live_note = format!("LIVE_AFTER_WATCH_{suffix}");

    let historical_response = post_polygon_notification_for_event_with_identifier(
        &client,
        &app.address,
        JETSTREAM_TEST_EVENT_TYPE,
        &historical_note,
        test_polygon(),
        JETSTREAM_TEST_DATE,
        JETSTREAM_WATCH_TEST_TIME,
    )
    .await;
    assert_status_ok_or_panic(historical_response, "historical notification").await;

    sleep(Duration::from_millis(300)).await;

    let mut watch_response = client
        .post(format!("{}/api/v1/watch", &app.address))
        .header("Content-Type", "application/json")
        .json(&json!({
            "event_type": JETSTREAM_TEST_EVENT_TYPE,
            "identifier": {
                "time": JETSTREAM_WATCH_TEST_TIME,
                "polygon": test_polygon(),
            }
        }))
        .send()
        .await
        .expect("failed to call watch endpoint");
    if watch_response.status() != StatusCode::OK {
        let status = watch_response.status();
        let body = watch_response
            .text()
            .await
            .unwrap_or_else(|_| "<failed to read body>".to_string());
        panic!("watch request failed with status {status}: {body}");
    }

    // Give the backend a brief moment to fully attach the subscription.
    sleep(Duration::from_millis(200)).await;

    let live_response = post_polygon_notification_for_event_with_identifier(
        &client,
        &app.address,
        JETSTREAM_TEST_EVENT_TYPE,
        &live_note,
        test_polygon(),
        JETSTREAM_TEST_DATE,
        JETSTREAM_WATCH_TEST_TIME,
    )
    .await;
    assert_status_ok_or_panic(live_response, "live notification").await;

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut observed = String::new();
    let mut saw_live_note = false;

    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let next_chunk_result = tokio::time::timeout(remaining, watch_response.chunk()).await;
        let next_chunk = match next_chunk_result {
            Err(_) => break,
            Ok(chunk_result) => chunk_result.expect("failed to read watch response chunk"),
        };

        match next_chunk {
            Some(chunk) => {
                observed.push_str(&String::from_utf8_lossy(&chunk));
                if observed.contains(&live_note) {
                    saw_live_note = true;
                    break;
                }
            }
            None => break,
        }
    }

    assert!(
        saw_live_note,
        "expected watch stream to include live note: {live_note}; observed: {observed}"
    );
    assert!(
        !observed.contains(&historical_note),
        "expected watch stream to exclude historical note: {historical_note}; observed: {observed}"
    );
}

#[tokio::test]
async fn jetstream_publish_after_replay_still_succeeds() {
    if !should_run_nats_tests() {
        return;
    }
    let _guard = JETSTREAM_TEST_LOCK.lock().await;

    let app = spawn_jetstream_test_app().await;
    let client = reqwest::Client::new();
    assert_jetstream_test_schema_is_available(&client, &app.address).await;
    let suffix = unique_suffix();

    // Step 1: publish and run a replay to exercise the same JetStream app instance.
    let replay_seed_note = format!("REPLAY_SEED_{suffix}");
    let replay_seed_response = post_polygon_notification_for_event_with_identifier(
        &client,
        &app.address,
        JETSTREAM_TEST_EVENT_TYPE,
        &replay_seed_note,
        test_polygon(),
        JETSTREAM_TEST_DATE,
        JETSTREAM_REPLAY_PUBLISH_TEST_TIME,
    )
    .await;
    assert_status_ok_or_panic(replay_seed_response, "replay-seed notification").await;

    let replay_response = client
        .post(format!("{}/api/v1/replay", &app.address))
        .header("Content-Type", "application/json")
        .json(&json!({
            "event_type": JETSTREAM_TEST_EVENT_TYPE,
            "identifier": {
                "time": JETSTREAM_REPLAY_PUBLISH_TEST_TIME,
                "polygon": test_polygon(),
            },
            "from_id": "1",
        }))
        .send()
        .await
        .expect("failed to call replay endpoint");
    assert_status_ok_or_panic(replay_response, "post-seed replay request").await;

    // Step 2: publish again immediately with a different subject and ensure storage still works.
    let post_replay_note = format!("POST_REPLAY_PUBLISH_{suffix}");
    let post_replay_response = post_polygon_notification_for_event_with_identifier(
        &client,
        &app.address,
        JETSTREAM_TEST_EVENT_TYPE,
        &post_replay_note,
        test_polygon(),
        JETSTREAM_TEST_DATE,
        JETSTREAM_POST_REPLAY_PUBLISH_TEST_TIME,
    )
    .await;
    assert_status_ok_or_panic(post_replay_response, "post-replay publish").await;
}

#[tokio::test]
async fn jetstream_schema_storage_policy_overrides_backend_defaults() {
    if !should_run_nats_tests() {
        return;
    }
    let _guard = JETSTREAM_TEST_LOCK.lock().await;

    let app = spawn_jetstream_test_app_with_backend_defaults(Some(5), Some(2048), Some("1h")).await;
    let client = reqwest::Client::new();
    assert_jetstream_test_schema_is_available(&client, &app.address).await;

    let note = format!("SCHEMA_POLICY_PRECEDENCE_{}", unique_suffix());
    let response = post_polygon_notification_for_event_with_identifier(
        &client,
        &app.address,
        JETSTREAM_TEST_EVENT_TYPE,
        &note,
        test_polygon(),
        JETSTREAM_TEST_DATE,
        "1510",
    )
    .await;
    assert_status_ok_or_panic(response, "precedence seed notification").await;

    let stream_config = fetch_stream_config("POLYGON_JS_TEST").await;
    assert_eq!(
        stream_config.max_messages, 5000,
        "schema storage_policy.max_messages should override backend default"
    );
    assert_eq!(
        stream_config.max_bytes, 67_108_864,
        "schema storage_policy.max_size=64Mi should override backend default"
    );
    assert_eq!(
        stream_config.max_age.as_secs(),
        7 * 24 * 60 * 60,
        "schema storage_policy.retention_time=7d should override backend default"
    );
    assert_eq!(
        stream_config.max_messages_per_subject, -1,
        "schema storage_policy.allow_duplicates=true should override backend default"
    );
    assert_eq!(
        stream_config.compression,
        Some(Compression::S2),
        "schema storage_policy.compression=true should override backend default"
    );
}
