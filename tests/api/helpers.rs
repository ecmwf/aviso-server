use std::sync::LazyLock;

use aviso_server::{
    configuration::LoggingSettings,
    configuration::get_configuration,
    startup::Application,
    telemetry::{get_subscriber, init_subscriber},
};

static TRACING: LazyLock<()> = LazyLock::new(|| {
    let default_filter_level = "warn".to_string();
    let default_format = "console".to_string();
    let logging_settings: Option<LoggingSettings> = LoggingSettings {
        level: default_filter_level.clone(),
        format: default_format.clone(),
    }
    .into();
    let subscriber_name = "test".to_string();
    if std::env::var("TEST_LOG").is_ok() {
        let subscriber =
            get_subscriber(subscriber_name, logging_settings.as_ref(), std::io::stdout);
        init_subscriber(subscriber);
    } else {
        let subscriber = get_subscriber(subscriber_name, logging_settings.as_ref(), std::io::sink);
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
