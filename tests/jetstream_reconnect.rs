use aviso_server::configuration::NotificationBackendSettings;
use aviso_server::notification_backend::NotificationBackend;
use aviso_server::notification_backend::jetstream::{JetStreamBackend, JetStreamConfig};
use std::process::Command;
use std::time::Duration;
use tokio::time::{sleep, timeout};

fn docker(args: &[&str]) -> String {
    let output = Command::new("docker").args(args).output().unwrap();
    assert!(
        output.status.success(),
        "docker {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

struct NatsContainer(String);

impl Drop for NatsContainer {
    fn drop(&mut self) {
        match Command::new("docker").args(["rm", "-f", &self.0]).output() {
            Ok(output) if output.status.success() => {}
            result => eprintln!("Failed to remove test container {}: {result:?}", self.0),
        }
    }
}

#[tokio::test]
#[ignore = "owns a disposable Docker NATS server; run scripts/test_jetstream_reconnect.sh"]
async fn bounded_and_unlimited_reconnect_after_outage() {
    let name = format!("aviso-reconnect-{}", uuid::Uuid::new_v4());
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let binding = format!("{address}:4222");
    // Pin the chosen port across restarts. A bind race fails Docker startup
    // before any client can connect to an unrelated service.
    drop(listener);
    let container = NatsContainer(docker(&[
        "create",
        "--name",
        &name,
        "--publish",
        &binding,
        "nats:2.14.6",
        "-js",
    ]));
    docker(&["start", &container.0]);
    assert_eq!(docker(&["port", &container.0, "4222/tcp"]), address);
    let settings: NotificationBackendSettings = serde_json::from_value(serde_json::json!({
        "kind": "jetstream",
        "jetstream": {
            "nats_url": format!("nats://{address}"),
            "timeout_seconds": 1,
            "retry_attempts": 30,
            "reconnect_delay_ms": 100
        }
    }))
    .unwrap();
    let config = JetStreamConfig::from_backend_settings(&settings).unwrap();
    let unset = JetStreamBackend::new(config.clone()).await.unwrap();
    let mut zero_config = config.clone();
    zero_config.max_reconnect_attempts = Some(0);
    let zero = JetStreamBackend::new(zero_config).await.unwrap();
    let mut bounded_config = config;
    bounded_config.max_reconnect_attempts = Some(3);
    let bounded = JetStreamBackend::new(bounded_config).await.unwrap();

    for backend in [&unset, &zero, &bounded] {
        backend.client.flush().await.unwrap();
        assert!(backend.connection_healthy());
    }
    docker(&["stop", "--time", "0", &container.0]);
    timeout(Duration::from_secs(10), async {
        while [&unset, &zero, &bounded]
            .iter()
            .any(|backend| backend.connection_healthy())
        {
            sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("all clients must observe the outage");

    // A closed command channel proves the bounded client exhausted its budget.
    timeout(Duration::from_secs(10), async {
        while bounded.client.flush().await.is_ok() {
            sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("bounded reconnect must terminate during the outage");
    sleep(Duration::from_secs(2)).await;
    assert!(!unset.connection_healthy());
    assert!(!zero.connection_healthy());

    docker(&["start", &container.0]);
    assert_eq!(docker(&["port", &container.0, "4222/tcp"]), address);
    timeout(Duration::from_secs(10), async {
        while !unset.connection_healthy() || !zero.connection_healthy() {
            sleep(Duration::from_millis(25)).await;
        }
        for backend in [&unset, &zero] {
            backend.jetstream.query_account().await.unwrap();
            backend.client.flush().await.unwrap();
        }
    })
    .await
    .expect("both unlimited clients must recover and serve JetStream requests");
    assert!(!bounded.connection_healthy());
    assert!(bounded.client.flush().await.is_err());
}
