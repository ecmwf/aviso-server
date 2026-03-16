use crate::helpers::{spawn_streaming_test_app, spawn_streaming_test_app_with_auth};
use serde_json::Value;

#[tokio::test]
async fn schema_list_does_not_expose_storage_policy() {
    let app = spawn_streaming_test_app().await;
    let response = reqwest::Client::new()
        .get(format!("{}/api/v1/schema", app.address))
        .send()
        .await
        .expect("failed to call schema list endpoint");

    assert_eq!(response.status().as_u16(), 200);
    let body = response
        .text()
        .await
        .expect("failed to read schema list response");
    assert!(
        !body.contains("\"storage_policy\""),
        "schema list response must not expose storage_policy; body: {body}"
    );
}

#[tokio::test]
async fn event_schema_does_not_expose_storage_policy() {
    let app = spawn_streaming_test_app().await;
    let response = reqwest::Client::new()
        .get(format!("{}/api/v1/schema/mars", app.address))
        .send()
        .await
        .expect("failed to call event schema endpoint");

    assert_eq!(response.status().as_u16(), 200);
    let body = response
        .text()
        .await
        .expect("failed to read event schema response");
    assert!(
        !body.contains("\"storage_policy\""),
        "event schema response must not expose storage_policy; body: {body}"
    );
}

#[tokio::test]
async fn event_schema_exposes_identifier_description_when_configured() {
    let app = spawn_streaming_test_app().await;
    let response = reqwest::Client::new()
        .get(format!("{}/api/v1/schema/mars", app.address))
        .send()
        .await
        .expect("failed to call event schema endpoint");

    assert_eq!(response.status().as_u16(), 200);
    let body: Value = response
        .json()
        .await
        .expect("failed to parse event schema response");

    let description = body
        .get("schema")
        .and_then(|schema| schema.get("identifier"))
        .and_then(|identifier| identifier.get("class"))
        .and_then(|field| field.get("description"))
        .and_then(|description| description.as_str());

    assert_eq!(
        description,
        Some("MARS class, for example od for operational data.")
    );
}

#[tokio::test]
async fn event_schema_omits_identifier_description_when_not_configured() {
    let app = spawn_streaming_test_app().await;
    let response = reqwest::Client::new()
        .get(format!("{}/api/v1/schema/mars", app.address))
        .send()
        .await
        .expect("failed to call event schema endpoint");

    assert_eq!(response.status().as_u16(), 200);
    let body: Value = response
        .json()
        .await
        .expect("failed to parse event schema response");

    let time_field = body
        .get("schema")
        .and_then(|schema| schema.get("identifier"))
        .and_then(|identifier| identifier.get("time"))
        .expect("schema response missing identifier.time");

    assert!(
        time_field.get("description").is_none(),
        "identifier.time description should be omitted when not configured"
    );
}

#[tokio::test]
async fn schema_list_is_public_when_auth_is_enabled() {
    let app = spawn_streaming_test_app_with_auth().await;
    let response = reqwest::Client::new()
        .get(format!("{}/api/v1/schema", app.address))
        .send()
        .await
        .expect("failed to call schema list endpoint");

    assert_eq!(response.status().as_u16(), 200);
}

#[tokio::test]
async fn event_schema_is_public_when_auth_is_enabled() {
    let app = spawn_streaming_test_app_with_auth().await;
    let response = reqwest::Client::new()
        .get(format!("{}/api/v1/schema/mars", app.address))
        .send()
        .await
        .expect("failed to call event schema endpoint");

    assert_eq!(response.status().as_u16(), 200);
}

#[tokio::test]
async fn schema_list_ignores_malformed_authorization_header() {
    let app = spawn_streaming_test_app_with_auth().await;
    let response = reqwest::Client::new()
        .get(format!("{}/api/v1/schema", app.address))
        .header("Authorization", "BadScheme garbage")
        .send()
        .await
        .expect("failed to call schema list endpoint");

    assert_eq!(response.status().as_u16(), 200);
}

#[tokio::test]
async fn event_schema_ignores_malformed_authorization_header() {
    let app = spawn_streaming_test_app_with_auth().await;
    let response = reqwest::Client::new()
        .get(format!("{}/api/v1/schema/mars", app.address))
        .header("Authorization", "BadScheme garbage")
        .send()
        .await
        .expect("failed to call event schema endpoint");

    assert_eq!(response.status().as_u16(), 200);
}
