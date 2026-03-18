use crate::helpers::spawn_streaming_test_app_with_auth;
use aviso_server::auth::JwtClaims;
use chrono::Utc;
use jsonwebtoken::{EncodingKey, Header, encode};
use serde_json::json;
use std::collections::HashMap;

fn ecpds_token(username: &str, roles: &[&str]) -> String {
    let claims = JwtClaims {
        sub: Some(username.to_string()),
        iss: None,
        exp: (Utc::now().timestamp() + 3600) as usize,
        iat: Some(Utc::now().timestamp() as usize),
        username: Some(username.to_string()),
        realm: Some("localrealm".to_string()),
        roles: roles.iter().map(|r| (*r).to_string()).collect(),
        attributes: HashMap::new(),
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret("test-jwt-secret".as_bytes()),
    )
    .expect("token must encode")
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

    assert_eq!(
        response.status(),
        reqwest::StatusCode::SERVICE_UNAVAILABLE
    );
}
