use crate::helpers::{mock_ecpds, spawn_streaming_test_app_with_auth, test_jwt};
use serde_json::json;

fn ecpds_token(username: &str, roles: &[&str]) -> String {
    test_jwt(username, roles)
}

fn diss_ecpds_watch_body(destination: &str) -> serde_json::Value {
    json!({
        "event_type": "dissemination_ecpds",
        "identifier": {
            "destination": destination,
            "class": "od"
        }
    })
}

fn diss_ecpds_replay_body(destination: &str) -> serde_json::Value {
    json!({
        "event_type": "dissemination_ecpds",
        "identifier": {
            "destination": destination,
            "class": "od"
        },
        "from_id": "1"
    })
}

#[tokio::test]
async fn watch_without_ecpds_plugin_allows_authenticated_user() {
    let app = spawn_streaming_test_app_with_auth().await;
    let client = reqwest::Client::new();
    let token = ecpds_token("reader-user", &["reader"]);

    let response = client
        .post(format!("{}/api/v1/watch", app.address))
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "event_type": "test_polygon_auth_any",
            "identifier": {
                "date": "20250706",
                "time": "1200",
                "polygon": "(0,0,0,1,1,1,0,0)"
            }
        }))
        .send()
        .await
        .expect("watch request should complete");

    assert_eq!(response.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn watch_ecpds_allows_user_with_valid_destination() {
    let app = spawn_streaming_test_app_with_auth().await;
    let client = reqwest::Client::new();
    let token = ecpds_token("ecpds-user", &["reader"]);

    let response = client
        .post(format!("{}/api/v1/watch", app.address))
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .json(&diss_ecpds_watch_body("CIP"))
        .send()
        .await
        .expect("watch request should complete");

    assert_eq!(response.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn watch_ecpds_denies_user_without_matching_destination() {
    let app = spawn_streaming_test_app_with_auth().await;
    let client = reqwest::Client::new();
    let token = ecpds_token("ecpds-user", &["reader"]);

    let response = client
        .post(format!("{}/api/v1/watch", app.address))
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .json(&diss_ecpds_watch_body("UNKNOWN"))
        .send()
        .await
        .expect("watch request should complete");

    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn watch_ecpds_denies_user_with_empty_destination_list() {
    let app = spawn_streaming_test_app_with_auth().await;
    let client = reqwest::Client::new();
    let token = ecpds_token("ecpds-noaccess", &["reader"]);

    let response = client
        .post(format!("{}/api/v1/watch", app.address))
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .json(&diss_ecpds_watch_body("CIP"))
        .send()
        .await
        .expect("watch request should complete");

    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn watch_ecpds_bypasses_check_for_admin() {
    let app = spawn_streaming_test_app_with_auth().await;
    let client = reqwest::Client::new();
    let token = ecpds_token("admin-user", &["admin"]);

    let response = client
        .post(format!("{}/api/v1/watch", app.address))
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .json(&diss_ecpds_watch_body("ANYTHING"))
        .send()
        .await
        .expect("watch request should complete");

    assert_eq!(response.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn watch_ecpds_unauthenticated_request_returns_401() {
    let app = spawn_streaming_test_app_with_auth().await;
    let client = reqwest::Client::new();

    let response = client
        .post(format!("{}/api/v1/watch", app.address))
        .header("Content-Type", "application/json")
        .json(&diss_ecpds_watch_body("CIP"))
        .send()
        .await
        .expect("watch request should complete");

    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn replay_ecpds_allows_user_with_valid_destination() {
    let app = spawn_streaming_test_app_with_auth().await;
    let client = reqwest::Client::new();
    let token = ecpds_token("ecpds-user", &["reader"]);

    let response = client
        .post(format!("{}/api/v1/replay", app.address))
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .json(&diss_ecpds_replay_body("CIP"))
        .send()
        .await
        .expect("replay request should complete");

    assert_eq!(response.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn replay_ecpds_denies_user_without_matching_destination() {
    let app = spawn_streaming_test_app_with_auth().await;
    let client = reqwest::Client::new();
    let token = ecpds_token("ecpds-user", &["reader"]);

    let response = client
        .post(format!("{}/api/v1/replay", app.address))
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .json(&diss_ecpds_replay_body("UNKNOWN"))
        .send()
        .await
        .expect("replay request should complete");

    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn watch_ecpds_returns_503_when_all_servers_fail() {
    let app = spawn_streaming_test_app_with_auth().await;
    let client = reqwest::Client::new();
    let token = ecpds_token("ecpds-unavailable", &["reader"]);

    let response = client
        .post(format!("{}/api/v1/watch", app.address))
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .json(&diss_ecpds_watch_body("CIP"))
        .send()
        .await
        .expect("watch request should complete");

    assert_eq!(response.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn watch_ecpds_caches_per_user_exactly_one_upstream_call() {
    let app = spawn_streaming_test_app_with_auth().await;
    let client = reqwest::Client::new();
    let username = "ecpds-user-cache-test";
    let token = ecpds_token(username, &["reader"]);
    let mock = mock_ecpds();
    let before = mock.count_for(username);

    for _ in 0..3 {
        let response = client
            .post(format!("{}/api/v1/watch", app.address))
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {token}"))
            .json(&diss_ecpds_watch_body("CIP"))
            .send()
            .await
            .expect("watch request should complete");
        assert_eq!(response.status(), reqwest::StatusCode::OK);
    }

    let after = mock.count_for(username);
    assert_eq!(
        after - before,
        1,
        "cache must coalesce 3 sequential requests for {username} into a single upstream fetch"
    );
}

#[tokio::test]
async fn watch_ecpds_concurrent_requests_coalesce() {
    let app = spawn_streaming_test_app_with_auth().await;
    let username = "ecpds-user-stampede-test";
    let token = ecpds_token(username, &["reader"]);
    let mock = mock_ecpds();
    let before = mock.count_for(username);

    let mut handles = Vec::new();
    for _ in 0..10 {
        let address = app.address.clone();
        let token = token.clone();
        handles.push(tokio::spawn(async move {
            reqwest::Client::new()
                .post(format!("{}/api/v1/watch", address))
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {token}"))
                .json(&diss_ecpds_watch_body("CIP"))
                .send()
                .await
        }));
    }

    for handle in handles {
        let response = handle
            .await
            .expect("task must join")
            .expect("watch request should complete");
        assert_eq!(response.status(), reqwest::StatusCode::OK);
    }

    let after = mock.count_for(username);
    assert_eq!(
        after - before,
        1,
        "single-flight must coalesce 10 concurrent requests for {username} into a single upstream fetch"
    );
}

#[tokio::test]
async fn watch_ecpds_username_with_special_chars_handled() {
    let app = spawn_streaming_test_app_with_auth().await;
    let client = reqwest::Client::new();
    let username = "u+s er&name";
    let token = ecpds_token(username, &["reader"]);
    let mock = mock_ecpds();
    let before = mock.count_for(username);

    let response = client
        .post(format!("{}/api/v1/watch", app.address))
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .json(&diss_ecpds_watch_body("CIP"))
        .send()
        .await
        .expect("watch request should complete");

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let after = mock.count_for(username);
    assert_eq!(
        after - before,
        1,
        "username with `+`, ` ` and `&` must round-trip URL-encoded \
         and reach the upstream identified by the original (decoded) value"
    );
}

#[tokio::test]
async fn notify_on_ecpds_protected_stream_does_not_invoke_ecpds_for_admin() {
    let app = spawn_streaming_test_app_with_auth().await;
    let client = reqwest::Client::new();
    let username = "admin-user-notify-bypass";
    let token = ecpds_token(username, &["admin"]);
    let mock = mock_ecpds();
    let before = mock.count_for(username);

    let response = client
        .post(format!("{}/api/v1/notification", app.address))
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "event_type": "dissemination_ecpds",
            "identifier": {
                "destination": "any-value-not-checked",
                "class": "od"
            },
            "payload": "irrelevant"
        }))
        .send()
        .await
        .expect("notify request should complete");

    assert!(
        response.status().is_success() || response.status().is_client_error(),
        "notify on ECPDS-protected stream must not 503; got {}",
        response.status()
    );
    let after = mock.count_for(username);
    assert_eq!(
        after, before,
        "notify must NOT invoke the ECPDS plugin under any policy"
    );
}

/// Non-admin counterpart to the admin notify-bypass test above. The ECPDS
/// plugin is read-only by design (only `enforce_ecpds_auth` callers in
/// watch/replay invoke it); admins additionally bypass the plugin even
/// on reads, so a passing admin test does not by itself prove that the
/// plugin is not consulted on writes. This case uses a non-admin
/// `producer` writer on a stream whose `auth.write_roles` grants that
/// role write access while keeping `plugins: ["ecpds"]` enabled. If a
/// future change accidentally wired `enforce_ecpds_auth` into the notify
/// path, the mock ECPDS would be hit (and likely deny since the user has
/// no destination list) rather than letting this assertion stay green.
#[tokio::test]
async fn notify_on_ecpds_protected_stream_does_not_invoke_ecpds_for_non_admin_writer() {
    let app = spawn_streaming_test_app_with_auth().await;
    let client = reqwest::Client::new();
    let username = "producer-user-notify-non-admin";
    let token = ecpds_token(username, &["producer"]);
    let mock = mock_ecpds();
    let before = mock.count_for(username);

    let response = client
        .post(format!("{}/api/v1/notification", app.address))
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "event_type": "dissemination_ecpds_writable",
            "identifier": {
                "destination": "any-value-not-checked",
                "class": "od"
            },
            "payload": { "note": "non-admin notify smoke" }
        }))
        .send()
        .await
        .expect("notify request should complete");

    let status = response.status();
    assert_ne!(
        status,
        reqwest::StatusCode::FORBIDDEN,
        "non-admin producer must be authorised to write by the test schema's \
         write_roles. A 403 here means either the schema or the role mapping \
         drifted; this test cannot prove notify ungating from a 403."
    );
    assert_ne!(
        status,
        reqwest::StatusCode::SERVICE_UNAVAILABLE,
        "notify by a non-admin producer on an ECPDS-protected writable stream \
         must not 503. A 503 means the plugin incorrectly ran on a write."
    );
    let after = mock.count_for(username);
    assert_eq!(
        after, before,
        "notify must NOT invoke the ECPDS plugin even for a non-admin writer"
    );
}
