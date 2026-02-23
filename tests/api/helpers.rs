use aviso_server::{
    configuration::ApplicationSettings,
    configuration::EventSchema,
    configuration::EventStoragePolicy,
    configuration::InMemorySettings,
    configuration::JetStreamSettings,
    configuration::LoggingSettings,
    configuration::NotificationBackendSettings,
    configuration::PayloadConfig,
    configuration::Settings,
    configuration::TopicConfig,
    configuration::WatchEndpointSettings,
    startup::Application,
    telemetry::{get_subscriber, init_subscriber},
};
use aviso_validators::ValidationRules;
use std::collections::HashMap;
use std::sync::LazyLock;
use std::sync::OnceLock;
use tokio::sync::OnceCell;
use tokio::task::JoinHandle;
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

static DEFAULT_SERVER: OnceCell<RunningServer> = OnceCell::const_new();
static STREAMING_SERVER: OnceCell<RunningServer> = OnceCell::const_new();
static TEST_GLOBAL_CONFIG: OnceLock<()> = OnceLock::new();

struct RunningServer {
    address: String,
    _shutdown_token: CancellationToken,
    _server_handle: JoinHandle<Result<(), std::io::Error>>,
}

#[derive(Clone)]
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
        storage_policy: None,
    }
}

fn build_test_polygon_js_schema() -> EventSchema {
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
            // Keep this distinct from runtime/default `polygon` base to avoid collisions.
            base: "polygon_js_test".to_string(),
            key_order: vec!["date".to_string(), "time".to_string()],
        }),
        endpoint: None,
        identifier,
        storage_policy: None,
    }
}

fn apply_jetstream_test_polygon_js_policy(schema: &mut HashMap<String, EventSchema>) {
    if let Some(test_polygon_js) = schema.get_mut("test_polygon_js") {
        // JetStream-specific tests need explicit policy values to validate
        // backend-default vs schema-override precedence.
        test_polygon_js.storage_policy = Some(EventStoragePolicy {
            retention_time: Some("7d".to_string()),
            max_messages: Some(5000),
            max_size: Some("64Mi".to_string()),
            allow_duplicates: Some(true),
            compression: Some(true),
        });
    }
}

fn build_test_polygon_optional_schema() -> EventSchema {
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
        vec![ValidationRules::PolygonHandler { required: false }],
    );

    EventSchema {
        payload: Some(PayloadConfig {
            allowed_types: vec!["String".to_string()],
            required: true,
        }),
        topic: Some(TopicConfig {
            base: "polygon_optional".to_string(),
            key_order: vec!["date".to_string(), "time".to_string()],
        }),
        endpoint: None,
        identifier,
        storage_policy: None,
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
        storage_policy: None,
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
        storage_policy: None,
    }
}

fn ensure_test_notification_schema(configuration: &mut Settings) {
    let schema = configuration
        .notification_schema
        .get_or_insert_with(HashMap::new);

    schema.insert("test_polygon".to_string(), build_test_polygon_schema());
    schema.insert(
        "test_polygon_optional".to_string(),
        build_test_polygon_optional_schema(),
    );
    schema.insert("mars".to_string(), build_mars_schema());
    schema.insert("dissemination".to_string(), build_dissemination_schema());
    schema.insert(
        "test_polygon_js".to_string(),
        build_test_polygon_js_schema(),
    );
}

fn set_streaming_test_notification_schema(configuration: &mut Settings) {
    configuration.notification_schema = Some(HashMap::new());
    ensure_test_notification_schema(configuration);
}

fn set_jetstream_test_notification_schema(configuration: &mut Settings) {
    configuration.notification_schema = Some(HashMap::new());
    ensure_test_notification_schema(configuration);
    if let Some(schema) = configuration.notification_schema.as_mut() {
        apply_jetstream_test_polygon_js_policy(schema);
    }
}

fn base_test_settings() -> Settings {
    Settings {
        application: ApplicationSettings {
            host: "127.0.0.1".to_string(),
            port: 0,
            base_url: "localhost:8000".to_string(),
            static_files_path: "./src/static".to_string(),
        },
        notification_backend: NotificationBackendSettings {
            kind: "in_memory".to_string(),
            in_memory: Some(InMemorySettings {
                max_history_per_topic: Some(100),
                max_topics: Some(500),
                enable_metrics: Some(false),
            }),
            jetstream: None,
        },
        logging: None,
        notification_schema: None,
        watch_endpoint: WatchEndpointSettings::default(),
    }
}

fn ensure_test_global_config_initialized() {
    TEST_GLOBAL_CONFIG.get_or_init(|| {
        let mut configuration = base_test_settings();
        ensure_test_notification_schema(&mut configuration);
        if let Some(schema) = configuration.notification_schema.as_mut() {
            apply_jetstream_test_polygon_js_policy(schema);
        }
        // This initialization is process-global (OnceLock-backed in production code),
        // so tests must install a deterministic superset schema exactly once.
        Settings::init_global_config(&configuration);
    });
}

async fn spawn_server(mut configuration: Settings) -> RunningServer {
    LazyLock::force(&TRACING);
    configuration.application.port = 0;
    ensure_test_global_config_initialized();
    let shutdown_token = CancellationToken::new();

    let application = Application::build(configuration.clone(), shutdown_token.clone())
        .await
        .expect("Failed to build server");
    let address = format!("http://127.0.0.1:{}", application.port());
    let server_handle = tokio::spawn(application.run_until_stopped());
    RunningServer {
        address,
        _shutdown_token: shutdown_token,
        _server_handle: server_handle,
    }
}

pub async fn spawn_app() -> TestApp {
    let running = DEFAULT_SERVER
        .get_or_init(|| async {
            let mut configuration = base_test_settings();
            ensure_test_notification_schema(&mut configuration);
            spawn_server(configuration).await
        })
        .await;

    TestApp {
        address: running.address.clone(),
    }
}

pub async fn spawn_streaming_test_app() -> TestApp {
    let running = STREAMING_SERVER
        .get_or_init(|| async {
            let mut configuration = base_test_settings();
            set_streaming_test_notification_schema(&mut configuration);

            spawn_server(configuration).await
        })
        .await;

    TestApp {
        address: running.address.clone(),
    }
}

pub async fn spawn_jetstream_test_app() -> TestApp {
    // JetStream backend uses async runtime-bound connection tasks.
    // Keep this server scoped to the current test runtime to avoid
    // cross-runtime channel closures between tests.
    let mut configuration = base_test_settings();
    configuration.notification_backend.kind = "jetstream".to_string();
    configuration.notification_backend.in_memory = None;
    configuration.notification_backend.jetstream = Some(JetStreamSettings {
        nats_url: Some(
            std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_string()),
        ),
        token: None,
        timeout_seconds: Some(10),
        retry_attempts: Some(3),
        max_messages: None,
        max_bytes: None,
        retention_time: None,
        storage_type: None,
        replicas: Some(1),
        retention_policy: None,
        discard_policy: None,
        enable_auto_reconnect: Some(true),
        max_reconnect_attempts: Some(5),
        reconnect_delay_ms: Some(200),
        publish_retry_attempts: Some(5),
        publish_retry_base_delay_ms: Some(150),
    });
    set_jetstream_test_notification_schema(&mut configuration);
    let running = spawn_server(configuration).await;
    TestApp {
        address: running.address,
    }
}

pub async fn spawn_jetstream_test_app_with_backend_defaults(
    max_messages: Option<i64>,
    max_bytes: Option<i64>,
    retention_time: Option<&str>,
) -> TestApp {
    // Keep this server runtime-local for the same reason as `spawn_jetstream_test_app`.
    let mut configuration = base_test_settings();
    configuration.notification_backend.kind = "jetstream".to_string();
    configuration.notification_backend.in_memory = None;
    configuration.notification_backend.jetstream = Some(JetStreamSettings {
        nats_url: Some(
            std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_string()),
        ),
        token: None,
        timeout_seconds: Some(10),
        retry_attempts: Some(3),
        max_messages,
        max_bytes,
        retention_time: retention_time.map(ToString::to_string),
        storage_type: None,
        replicas: Some(1),
        retention_policy: None,
        discard_policy: None,
        enable_auto_reconnect: Some(true),
        max_reconnect_attempts: Some(5),
        reconnect_delay_ms: Some(200),
        publish_retry_attempts: Some(5),
        publish_retry_base_delay_ms: Some(150),
    });
    set_jetstream_test_notification_schema(&mut configuration);
    let running = spawn_server(configuration).await;
    TestApp {
        address: running.address,
    }
}
