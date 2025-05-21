use crate::helpers::spawn_app;

#[tokio::test]
async fn health_check_works() {
    let app = spawn_app().await;
    let client = reqwest::Client::new();

    let response = client
        .get(&format!("{}/health", &app.address))
        .send()
        .await
        .expect("Failed to send request");

    assert!(response.status().is_success());
}
