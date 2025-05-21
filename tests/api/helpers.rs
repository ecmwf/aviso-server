use aviso_server::configuration::get_configuration;
use aviso_server::startup::Application;
use aviso_server::telemetry::{get_subscriber, init_subscriber};
use std::sync::LazyLock;

static TRACING: LazyLock<()> = LazyLock::new(|| {
    let default_filter_level = "info".to_string();
    let subscriber_name = "test".to_string();
    if std::env::var("TEST_LOG").is_ok() {
        let subscriber = get_subscriber(subscriber_name, default_filter_level, std::io::stdout);
        init_subscriber(subscriber);
    } else {
        let subscriber = get_subscriber(subscriber_name, default_filter_level, std::io::sink);
        init_subscriber(subscriber);
    }
});
pub struct TestApp {
    pub address: String,
}

pub async fn spawn_app() -> TestApp {
    LazyLock::force(&TRACING);

    let configuration = {
        let c = get_configuration().expect("Failed to read configuration");
        // overrides should be set here
        c
    };

    let application = Application::build(configuration.clone())
        .await
        .expect("Failed to build server");
    let address = format!("http://127.0.0.1:{}", application.port());
    let _ = tokio::spawn(application.run_until_stopped());
    TestApp { address }
}
