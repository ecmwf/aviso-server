use crate::helpers::spawn_streaming_test_app;

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
