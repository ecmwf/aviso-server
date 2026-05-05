// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use crate::helpers::{
    spawn_streaming_test_app_with_auth, spawn_streaming_test_app_with_trusted_proxy_auth, test_jwt,
    test_jwt_with_secret_and_exp,
};
use reqwest::header::WWW_AUTHENTICATE;
use serde_json::json;

fn auth_token_with_secret_and_exp(
    username: &str,
    roles: &[&str],
    secret: &str,
    exp_offset_seconds: i64,
) -> String {
    test_jwt_with_secret_and_exp(username, roles, secret, exp_offset_seconds)
}

fn auth_token(username: &str, roles: &[&str]) -> String {
    test_jwt(username, roles)
}

async fn post_notify(
    client: &reqwest::Client,
    address: &str,
    event_type: &str,
    authorization_header: Option<&str>,
) -> reqwest::Response {
    let mut request = client
        .post(format!("{}/api/v1/notification", address))
        .header("Content-Type", "application/json");

    if let Some(header_value) = authorization_header {
        request = request.header("Authorization", header_value);
    }

    request
        .json(&json!({
            "event_type": event_type,
            "identifier": {
                "date": "20250706",
                "time": "1200",
                "polygon": "(0,0,0,1,1,1,0,0)"
            },
            "payload": {"source": "smoke"}
        }))
        .send()
        .await
        .expect("notify request should complete")
}

#[tokio::test]
async fn watch_auth_required_rejects_missing_token() {
    let app = spawn_streaming_test_app_with_auth().await;
    let client = reqwest::Client::new();

    let response = client
        .post(format!("{}/api/v1/watch", app.address))
        .header("Content-Type", "application/json")
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

    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    let challenges: Vec<_> = response
        .headers()
        .get_all(WWW_AUTHENTICATE)
        .into_iter()
        .filter_map(|value| value.to_str().ok())
        .collect();
    assert_eq!(challenges, vec!["Bearer", "Basic"]);
}

#[tokio::test]
async fn watch_auth_required_allows_any_authenticated_role_when_roles_not_set() {
    let app = spawn_streaming_test_app_with_auth().await;
    let client = reqwest::Client::new();
    let token = auth_token("reader-user", &["reader"]);

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
async fn watch_auth_required_enforces_read_roles() {
    let app = spawn_streaming_test_app_with_auth().await;
    let client = reqwest::Client::new();
    let token = auth_token("reader-user", &["reader"]);

    let response = client
        .post(format!("{}/api/v1/watch", app.address))
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "event_type": "test_polygon_auth_admin",
            "identifier": {
                "date": "20250706",
                "time": "1200",
                "polygon": "(0,0,0,1,1,1,0,0)"
            }
        }))
        .send()
        .await
        .expect("watch request should complete");

    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn replay_auth_required_allows_authorized_role() {
    let app = spawn_streaming_test_app_with_auth().await;
    let client = reqwest::Client::new();
    let token = auth_token("admin-user", &["admin"]);

    let response = client
        .post(format!("{}/api/v1/replay", app.address))
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "event_type": "test_polygon_auth_admin",
            "identifier": {
                "date": "20250706",
                "time": "1200",
                "polygon": "(0,0,0,1,1,1,0,0)"
            },
            "from_id": "1"
        }))
        .send()
        .await
        .expect("replay request should complete");

    assert_eq!(response.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn watch_auth_optional_allows_anonymous() {
    let app = spawn_streaming_test_app_with_auth().await;
    let client = reqwest::Client::new();

    let response = client
        .post(format!("{}/api/v1/watch", app.address))
        .header("Content-Type", "application/json")
        .json(&json!({
            "event_type": "test_polygon_auth_optional",
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
async fn notify_auth_required_rejects_missing_token() {
    let app = spawn_streaming_test_app_with_auth().await;
    let client = reqwest::Client::new();

    let response = post_notify(&client, &app.address, "test_polygon_auth_any", None).await;

    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn notify_auth_required_allows_authorized_role() {
    let app = spawn_streaming_test_app_with_auth().await;
    let client = reqwest::Client::new();
    let token = auth_token("admin-user", &["admin"]);

    let response = post_notify(
        &client,
        &app.address,
        "test_polygon_auth_admin",
        Some(&format!("Bearer {token}")),
    )
    .await;

    assert_eq!(response.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn notify_auth_required_rejects_unauthorized_role() {
    let app = spawn_streaming_test_app_with_auth().await;
    let client = reqwest::Client::new();
    let token = auth_token("reader-user", &["reader"]);

    let response = post_notify(
        &client,
        &app.address,
        "test_polygon_auth_admin",
        Some(&format!("Bearer {token}")),
    )
    .await;

    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn notify_auth_optional_allows_anonymous() {
    let app = spawn_streaming_test_app_with_auth().await;
    let client = reqwest::Client::new();

    let response = post_notify(&client, &app.address, "test_polygon_auth_optional", None).await;

    assert_eq!(response.status(), reqwest::StatusCode::OK);
}

// --- Read/write role separation tests ---

#[tokio::test]
async fn notify_write_defaults_to_admin_only_when_write_roles_absent() {
    let app = spawn_streaming_test_app_with_auth().await;
    let client = reqwest::Client::new();
    // "reader" is authenticated but not admin → rejected for write on auth_any (no write_roles).
    let token = auth_token("reader-user", &["reader"]);

    let response = post_notify(
        &client,
        &app.address,
        "test_polygon_auth_any",
        Some(&format!("Bearer {token}")),
    )
    .await;

    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn notify_write_allowed_with_explicit_write_roles() {
    let app = spawn_streaming_test_app_with_auth().await;
    let client = reqwest::Client::new();
    let token = auth_token("producer-user", &["producer"]);

    let response = post_notify(
        &client,
        &app.address,
        "test_polygon_auth_write",
        Some(&format!("Bearer {token}")),
    )
    .await;

    assert_eq!(response.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn notify_write_rejected_without_matching_write_role() {
    let app = spawn_streaming_test_app_with_auth().await;
    let client = reqwest::Client::new();
    let token = auth_token("reader-user", &["reader"]);

    let response = post_notify(
        &client,
        &app.address,
        "test_polygon_auth_write",
        Some(&format!("Bearer {token}")),
    )
    .await;

    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn notify_admin_can_always_write_regardless_of_write_roles() {
    let app = spawn_streaming_test_app_with_auth().await;
    let client = reqwest::Client::new();
    let token = auth_token("admin-user", &["admin"]);

    let response = post_notify(
        &client,
        &app.address,
        "test_polygon_auth_write",
        Some(&format!("Bearer {token}")),
    )
    .await;

    assert_eq!(response.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn watch_read_allowed_for_any_authenticated_when_read_roles_absent() {
    let app = spawn_streaming_test_app_with_auth().await;
    let client = reqwest::Client::new();
    // auth_write has read_roles: None → any authenticated user can read.
    let token = auth_token("reader-user", &["reader"]);

    let response = client
        .post(format!("{}/api/v1/watch", app.address))
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "event_type": "test_polygon_auth_write",
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
async fn watch_auth_required_accepts_basic_credentials_in_direct_mode() {
    let app = spawn_streaming_test_app_with_auth().await;
    let client = reqwest::Client::new();

    let response = client
        .post(format!("{}/api/v1/watch", app.address))
        .header("Content-Type", "application/json")
        .header("Authorization", "Basic cmVhZGVyLXVzZXI6cmVhZGVyLXBhc3M=")
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
async fn watch_auth_required_rejects_expired_bearer_token_in_direct_mode() {
    let app = spawn_streaming_test_app_with_auth().await;
    let client = reqwest::Client::new();
    let token =
        auth_token_with_secret_and_exp("reader-user", &["reader"], "test-jwt-secret", -7_200);

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

    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn watch_auth_required_rejects_wrong_signature_bearer_token_in_direct_mode() {
    let app = spawn_streaming_test_app_with_auth().await;
    let client = reqwest::Client::new();
    let token = auth_token_with_secret_and_exp("reader-user", &["reader"], "wrong-secret", 3600);

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

    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn trusted_proxy_watch_accepts_valid_identity_headers() {
    let app = spawn_streaming_test_app_with_trusted_proxy_auth().await;
    let client = reqwest::Client::new();
    let token = auth_token("reader-user", &["reader"]);

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
async fn trusted_proxy_watch_rejects_non_bearer_authorization() {
    let app = spawn_streaming_test_app_with_trusted_proxy_auth().await;
    let client = reqwest::Client::new();

    let response = client
        .post(format!("{}/api/v1/watch", app.address))
        .header("Content-Type", "application/json")
        .header("Authorization", "Basic cmVhZGVyLXVzZXI6cmVhZGVyLXBhc3M=")
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

    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    let challenges: Vec<_> = response
        .headers()
        .get_all(WWW_AUTHENTICATE)
        .into_iter()
        .filter_map(|value| value.to_str().ok())
        .collect();
    assert_eq!(challenges, vec!["Bearer"]);
}

#[tokio::test]
async fn trusted_proxy_admin_route_requires_admin_role() {
    let app = spawn_streaming_test_app_with_trusted_proxy_auth().await;
    let client = reqwest::Client::new();
    let token = auth_token("reader-user", &["reader"]);

    let response = client
        .delete(format!("{}/api/v1/admin/notification/mars@1", app.address))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("admin request should complete");

    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn trusted_proxy_admin_route_allows_admin_role() {
    let app = spawn_streaming_test_app_with_trusted_proxy_auth().await;
    let client = reqwest::Client::new();
    let token = auth_token("admin-user", &["admin"]);

    let response = client
        .delete(format!("{}/api/v1/admin/notification/mars@1", app.address))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("admin request should complete");

    // Route authorization passed; missing sequence returns 404.
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn direct_mode_admin_route_allows_admin_credentials() {
    let app = spawn_streaming_test_app_with_auth().await;
    let client = reqwest::Client::new();

    let response = client
        .delete(format!("{}/api/v1/admin/notification/mars@1", app.address))
        .header("Authorization", "Basic YWRtaW4tdXNlcjphZG1pbi1wYXNz")
        .send()
        .await
        .expect("admin request should complete");

    // Route authorization passed; missing sequence returns 404.
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
}
