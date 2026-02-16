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

fn build_test_polygon_schema() -> EventSchema {
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

    EventSchema {
        payload: Some(PayloadConfig {
            allowed_types: vec!["String".to_string()],
            required: true,
        }),
        topic: Some(TopicConfig {
            base: "polygon".to_string(),
            key_order: vec!["date".to_string(), "time".to_string()],
        }),
        endpoint: None,
        identifier,
    }
}

fn build_mars_schema() -> EventSchema {
    let mut identifier = HashMap::new();
    identifier.insert(
        "class".to_string(),
        vec![ValidationRules::StringHandler {
            max_length: Some(2),
            required: true,
        }],
    );
    identifier.insert(
        "expver".to_string(),
        vec![ValidationRules::ExpverHandler {
            default: Some("0001".to_string()),
            required: false,
        }],
    );
    identifier.insert(
        "domain".to_string(),
        vec![ValidationRules::EnumHandler {
            values: vec![
                "a".to_string(),
                "b".to_string(),
                "c".to_string(),
                "d".to_string(),
                "e".to_string(),
                "f".to_string(),
                "g".to_string(),
                "h".to_string(),
                "i".to_string(),
                "j".to_string(),
                "k".to_string(),
                "l".to_string(),
                "m".to_string(),
                "n".to_string(),
                "o".to_string(),
                "p".to_string(),
                "q".to_string(),
                "r".to_string(),
                "s".to_string(),
                "t".to_string(),
                "u".to_string(),
                "v".to_string(),
                "w".to_string(),
                "x".to_string(),
                "y".to_string(),
                "z".to_string(),
            ],
            required: false,
        }],
    );
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
        "stream".to_string(),
        vec![ValidationRules::StringHandler {
            max_length: None,
            required: false,
        }],
    );
    identifier.insert(
        "step".to_string(),
        vec![ValidationRules::IntHandler {
            range: Some([0, 100000]),
            required: false,
        }],
    );

    EventSchema {
        payload: Some(PayloadConfig {
            allowed_types: vec!["String".to_string(), "NoneType".to_string()],
            required: false,
        }),
        topic: Some(TopicConfig {
            base: "mars".to_string(),
            key_order: vec![
                "class".to_string(),
                "expver".to_string(),
                "domain".to_string(),
                "date".to_string(),
                "time".to_string(),
                "stream".to_string(),
                "step".to_string(),
            ],
        }),
        endpoint: None,
        identifier,
    }
}

fn build_dissemination_schema() -> EventSchema {
    let mut identifier = HashMap::new();
    identifier.insert(
        "destination".to_string(),
        vec![ValidationRules::StringHandler {
            max_length: None,
            required: true,
        }],
    );
    identifier.insert(
        "target".to_string(),
        vec![ValidationRules::StringHandler {
            max_length: None,
            required: false,
        }],
    );
    identifier.insert(
        "class".to_string(),
        vec![ValidationRules::StringHandler {
            max_length: Some(2),
            required: true,
        }],
    );
    identifier.insert(
        "expver".to_string(),
        vec![ValidationRules::ExpverHandler {
            default: Some("0001".to_string()),
            required: false,
        }],
    );
    identifier.insert(
        "domain".to_string(),
        vec![ValidationRules::EnumHandler {
            values: vec![
                "a".to_string(),
                "b".to_string(),
                "c".to_string(),
                "d".to_string(),
                "e".to_string(),
                "f".to_string(),
                "g".to_string(),
                "h".to_string(),
                "i".to_string(),
                "j".to_string(),
                "k".to_string(),
                "l".to_string(),
                "m".to_string(),
                "n".to_string(),
                "o".to_string(),
                "p".to_string(),
                "q".to_string(),
                "r".to_string(),
                "s".to_string(),
                "t".to_string(),
                "u".to_string(),
                "v".to_string(),
                "w".to_string(),
                "x".to_string(),
                "y".to_string(),
                "z".to_string(),
            ],
            required: false,
        }],
    );
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
        "stream".to_string(),
        vec![ValidationRules::StringHandler {
            max_length: None,
            required: false,
        }],
    );
    identifier.insert(
        "step".to_string(),
        vec![ValidationRules::IntHandler {
            range: Some([0, 100000]),
            required: false,
        }],
    );

    EventSchema {
        payload: Some(PayloadConfig {
            allowed_types: vec![
                "String".to_string(),
                "HashMap".to_string(),
                "CloudEvent".to_string(),
            ],
            required: true,
        }),
        topic: Some(TopicConfig {
            base: "diss".to_string(),
            key_order: vec![
                "destination".to_string(),
                "target".to_string(),
                "class".to_string(),
                "expver".to_string(),
                "domain".to_string(),
                "date".to_string(),
                "time".to_string(),
                "stream".to_string(),
                "step".to_string(),
            ],
        }),
        endpoint: None,
        identifier,
    }
}

fn ensure_test_notification_schema(configuration: &mut Settings) {
    let schema = configuration
        .notification_schema
        .get_or_insert_with(HashMap::new);

    schema.insert("test_polygon".to_string(), build_test_polygon_schema());
}

fn set_streaming_test_notification_schema(configuration: &mut Settings) {
    let mut schema = HashMap::new();
    schema.insert("test_polygon".to_string(), build_test_polygon_schema());
    schema.insert("mars".to_string(), build_mars_schema());
    schema.insert("dissemination".to_string(), build_dissemination_schema());
    configuration.notification_schema = Some(schema);
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
        set_streaming_test_notification_schema(configuration);
    })
    .await
}
