use aviso_server::{
    configuration::InMemorySettings,
    configuration::LoggingSettings,
    configuration::Settings,
    configuration::get_configuration,
    startup::Application,
    telemetry::{get_subscriber, init_subscriber},
};
use std::sync::LazyLock;
use tokio_util::sync::CancellationToken;

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
    let mut configuration = {
        // overrides should be set here
        get_configuration().expect("Failed to read configuration")
    };
    configuration.application.port = 0;

    aviso_server::configuration::Settings::init_global_config(&configuration.clone());
    let shutdown_token = CancellationToken::new();

    let application = Application::build(configuration.clone(), shutdown_token.clone())
        .await
        .expect("Failed to build server");
    let address = format!("http://127.0.0.1:{}", application.port());
    std::mem::drop(tokio::spawn(application.run_until_stopped()));
    TestApp { address }
}

pub async fn spawn_app_with_config<F>(configure: F) -> TestApp
where
    F: FnOnce(&mut Settings),
{
    LazyLock::force(&TRACING);
    let mut configuration = get_configuration().expect("Failed to read configuration");
    configuration.application.port = 0;
    configure(&mut configuration);

    aviso_server::configuration::Settings::init_global_config(&configuration.clone());
    let shutdown_token = CancellationToken::new();

    let application = Application::build(configuration.clone(), shutdown_token.clone())
        .await
        .expect("Failed to build server");
    let address = format!("http://127.0.0.1:{}", application.port());
    std::mem::drop(tokio::spawn(application.run_until_stopped()));
    TestApp { address }
}

pub async fn spawn_streaming_test_app() -> TestApp {
    spawn_app_with_config(|configuration| {
        configuration.notification_backend.kind = "in_memory".to_string();
        configuration.notification_backend.jetstream = None;
        configuration.notification_backend.in_memory = Some(InMemorySettings {
            max_history_per_topic: Some(100),
            max_topics: Some(500),
            enable_metrics: Some(false),
        });
    })
    .await
}
