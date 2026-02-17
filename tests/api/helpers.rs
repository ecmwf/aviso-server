use aviso_server::{
    configuration::EventSchema,
    configuration::InMemorySettings,
    configuration::LoggingSettings,
    configuration::PayloadConfig,
    configuration::Settings,
    configuration::TopicConfig,
    configuration::get_configuration,
    startup::Application,
    telemetry::{get_subscriber, init_subscriber},
};
use aviso_validators::ValidationRules;
use std::collections::HashMap;
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

fn ensure_test_notification_schema(configuration: &mut Settings) {
    let schema = configuration
        .notification_schema
        .get_or_insert_with(HashMap::new);

    let mut identifier = HashMap::new();
    identifier.insert(
        "date".to_string(),
        vec![ValidationRules::DateHandler {
            canonical_format: "%Y%m%d".to_string(),
            required: false,
        }],
    );
    identifier.insert(
        "time".to_string(),
        vec![ValidationRules::TimeHandler { required: false }],
    );
    identifier.insert(
        "polygon".to_string(),
        vec![ValidationRules::PolygonHandler { required: true }],
    );

    schema.insert(
        "test_polygon".to_string(),
        EventSchema {
            payload: Some(PayloadConfig {
                allowed_types: vec!["String".to_string()],
                required: true,
            }),
            topic: Some(TopicConfig {
                base: "polygon".to_string(),
                separator: ".".to_string(),
                key_order: vec!["date".to_string(), "time".to_string()],
            }),
            endpoint: None,
            identifier,
        },
    );
}

pub async fn spawn_app() -> TestApp {
    LazyLock::force(&TRACING);
    let mut configuration = {
        // overrides should be set here
        get_configuration().expect("Failed to read configuration")
    };
    configuration.application.port = 0;
    ensure_test_notification_schema(&mut configuration);

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
    ensure_test_notification_schema(&mut configuration);

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
