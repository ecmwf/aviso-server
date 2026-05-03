// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use actix_web::{App, HttpRequest, HttpResponse, HttpServer, http::header, web};
use aviso_server::{
    auth::client::is_jwt_like,
    configuration::ApplicationSettings,
    configuration::AuthSettings,
    configuration::EventSchema,
    configuration::EventStoragePolicy,
    configuration::IdentifierFieldConfig,
    configuration::InMemorySettings,
    configuration::JetStreamSettings,
    configuration::LoggingSettings,
    configuration::MetricsSettings,
    configuration::NotificationBackendSettings,
    configuration::PayloadConfig,
    configuration::Settings,
    configuration::StreamAuthConfig,
    configuration::TopicConfig,
    configuration::WatchEndpointSettings,
    startup::Application,
    telemetry::{get_subscriber, init_subscriber},
};
use aviso_validators::ValidationRules;
use chrono::Utc;
use jsonwebtoken::{EncodingKey, Header, encode};
use std::collections::HashMap;
use std::net::TcpListener;
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
static STREAMING_AUTH_SERVER: OnceCell<RunningServer> = OnceCell::const_new();
static STREAMING_TRUSTED_PROXY_SERVER: OnceCell<RunningServer> = OnceCell::const_new();
static TEST_GLOBAL_CONFIG: OnceLock<()> = OnceLock::new();

#[cfg(feature = "ecmwf")]
static MOCK_ECPDS_URL: LazyLock<String> = LazyLock::new(start_sync_mock_ecpds_server);

struct RunningServer {
    address: String,
    _shutdown_token: CancellationToken,
    _server_handle: JoinHandle<Result<(), std::io::Error>>,
    _auth_server_handle: Option<JoinHandle<std::io::Result<()>>>,
}

#[derive(Clone)]
pub struct TestApp {
    pub address: String,
}

fn build_test_polygon_schema() -> EventSchema {
    let mut identifier = HashMap::new();
    identifier.insert(
        "date".to_string(),
        IdentifierFieldConfig::with_rule(ValidationRules::DateHandler {
            canonical_format: "%Y%m%d".to_string(),
            required: false,
        }),
    );
    identifier.insert(
        "time".to_string(),
        IdentifierFieldConfig::with_rule(ValidationRules::TimeHandler { required: false }),
    );
    identifier.insert(
        "polygon".to_string(),
        IdentifierFieldConfig::with_rule(ValidationRules::PolygonHandler { required: true }),
    );

    EventSchema {
        payload: Some(PayloadConfig { required: true }),
        topic: Some(TopicConfig {
            base: "polygon".to_string(),
            key_order: vec!["date".to_string(), "time".to_string()],
        }),
        endpoint: None,
        identifier,
        storage_policy: None,
        auth: None,
    }
}

fn build_test_polygon_js_schema() -> EventSchema {
    let mut identifier = HashMap::new();
    identifier.insert(
        "date".to_string(),
        IdentifierFieldConfig::with_rule(ValidationRules::DateHandler {
            canonical_format: "%Y%m%d".to_string(),
            required: false,
        }),
    );
    identifier.insert(
        "time".to_string(),
        IdentifierFieldConfig::with_rule(ValidationRules::TimeHandler { required: false }),
    );
    identifier.insert(
        "polygon".to_string(),
        IdentifierFieldConfig::with_rule(ValidationRules::PolygonHandler { required: true }),
    );

    EventSchema {
        payload: Some(PayloadConfig { required: true }),
        topic: Some(TopicConfig {
            // Keep this distinct from runtime/default `polygon` base to avoid collisions.
            base: "polygon_js_test".to_string(),
            key_order: vec!["date".to_string(), "time".to_string()],
        }),
        endpoint: None,
        identifier,
        storage_policy: None,
        auth: None,
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
        IdentifierFieldConfig::with_rule(ValidationRules::DateHandler {
            canonical_format: "%Y%m%d".to_string(),
            required: false,
        }),
    );
    identifier.insert(
        "time".to_string(),
        IdentifierFieldConfig::with_rule(ValidationRules::TimeHandler { required: false }),
    );
    identifier.insert(
        "polygon".to_string(),
        IdentifierFieldConfig::with_rule(ValidationRules::PolygonHandler { required: false }),
    );

    EventSchema {
        payload: Some(PayloadConfig { required: true }),
        topic: Some(TopicConfig {
            base: "polygon_optional".to_string(),
            key_order: vec!["date".to_string(), "time".to_string()],
        }),
        endpoint: None,
        identifier,
        storage_policy: None,
        auth: None,
    }
}

fn build_mars_schema() -> EventSchema {
    let mut identifier = HashMap::new();
    identifier.insert(
        "class".to_string(),
        IdentifierFieldConfig::with_description(
            "MARS class, for example od for operational data.",
            ValidationRules::StringHandler {
                max_length: Some(2),
                required: true,
            },
        ),
    );
    identifier.insert(
        "expver".to_string(),
        IdentifierFieldConfig::with_rule(ValidationRules::ExpverHandler {
            default: Some("0001".to_string()),
            required: false,
        }),
    );
    identifier.insert(
        "domain".to_string(),
        IdentifierFieldConfig::with_rule(ValidationRules::EnumHandler {
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
        }),
    );
    identifier.insert(
        "date".to_string(),
        IdentifierFieldConfig::with_rule(ValidationRules::DateHandler {
            canonical_format: "%Y%m%d".to_string(),
            required: false,
        }),
    );
    identifier.insert(
        "time".to_string(),
        IdentifierFieldConfig::with_rule(ValidationRules::TimeHandler { required: false }),
    );
    identifier.insert(
        "stream".to_string(),
        IdentifierFieldConfig::with_rule(ValidationRules::StringHandler {
            max_length: None,
            required: false,
        }),
    );
    identifier.insert(
        "step".to_string(),
        IdentifierFieldConfig::with_rule(ValidationRules::IntHandler {
            range: Some([0, 100000]),
            required: false,
        }),
    );

    EventSchema {
        payload: Some(PayloadConfig { required: false }),
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
        auth: None,
    }
}

fn build_dissemination_schema() -> EventSchema {
    let mut identifier = HashMap::new();
    identifier.insert(
        "destination".to_string(),
        IdentifierFieldConfig::with_rule(ValidationRules::StringHandler {
            max_length: None,
            required: true,
        }),
    );
    identifier.insert(
        "target".to_string(),
        IdentifierFieldConfig::with_rule(ValidationRules::StringHandler {
            max_length: None,
            required: false,
        }),
    );
    identifier.insert(
        "class".to_string(),
        IdentifierFieldConfig::with_rule(ValidationRules::StringHandler {
            max_length: Some(2),
            required: true,
        }),
    );
    identifier.insert(
        "expver".to_string(),
        IdentifierFieldConfig::with_rule(ValidationRules::ExpverHandler {
            default: Some("0001".to_string()),
            required: false,
        }),
    );
    identifier.insert(
        "domain".to_string(),
        IdentifierFieldConfig::with_rule(ValidationRules::EnumHandler {
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
        }),
    );
    identifier.insert(
        "date".to_string(),
        IdentifierFieldConfig::with_rule(ValidationRules::DateHandler {
            canonical_format: "%Y%m%d".to_string(),
            required: false,
        }),
    );
    identifier.insert(
        "time".to_string(),
        IdentifierFieldConfig::with_rule(ValidationRules::TimeHandler { required: false }),
    );
    identifier.insert(
        "stream".to_string(),
        IdentifierFieldConfig::with_rule(ValidationRules::StringHandler {
            max_length: None,
            required: false,
        }),
    );
    identifier.insert(
        "step".to_string(),
        IdentifierFieldConfig::with_rule(ValidationRules::IntHandler {
            range: Some([0, 100000]),
            required: false,
        }),
    );

    EventSchema {
        payload: Some(PayloadConfig { required: true }),
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
        auth: None,
    }
}

fn build_extreme_event_schema() -> EventSchema {
    let mut identifier = HashMap::new();
    identifier.insert(
        "region".to_string(),
        IdentifierFieldConfig::with_rule(ValidationRules::EnumHandler {
            values: vec![
                "north".to_string(),
                "south".to_string(),
                "east".to_string(),
                "west".to_string(),
            ],
            required: false,
        }),
    );
    identifier.insert(
        "run_time".to_string(),
        IdentifierFieldConfig::with_rule(ValidationRules::TimeHandler { required: false }),
    );
    identifier.insert(
        "severity".to_string(),
        IdentifierFieldConfig::with_rule(ValidationRules::IntHandler {
            range: Some([1, 7]),
            required: false,
        }),
    );
    identifier.insert(
        "anomaly".to_string(),
        IdentifierFieldConfig::with_rule(ValidationRules::FloatHandler {
            range: Some([0.0, 200.0]),
            required: false,
        }),
    );

    EventSchema {
        payload: Some(PayloadConfig { required: false }),
        topic: Some(TopicConfig {
            base: "extreme".to_string(),
            key_order: vec![
                "region".to_string(),
                "run_time".to_string(),
                "severity".to_string(),
                "anomaly".to_string(),
            ],
        }),
        endpoint: None,
        identifier,
        storage_policy: None,
        auth: None,
    }
}

fn ensure_test_notification_schema(configuration: &mut Settings, include_auth_schemas: bool) {
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
    schema.insert("extreme".to_string(), build_extreme_event_schema());
    schema.insert(
        "test_polygon_js".to_string(),
        build_test_polygon_js_schema(),
    );

    if include_auth_schemas {
        let mut auth_any = build_test_polygon_schema();
        auth_any
            .topic
            .as_mut()
            .expect("test schema must have topic")
            .base = "polygon_auth_any".to_string();
        auth_any.auth = Some(StreamAuthConfig {
            required: true,
            read_roles: None,
            write_roles: None,
            plugins: None,
        });
        schema.insert("test_polygon_auth_any".to_string(), auth_any);

        // Read restricted to admin role; write defaults to admin-only (no write_roles).
        let mut auth_admin = build_test_polygon_schema();
        auth_admin
            .topic
            .as_mut()
            .expect("test schema must have topic")
            .base = "polygon_auth_admin".to_string();
        auth_admin.auth = Some(StreamAuthConfig {
            required: true,
            read_roles: Some(HashMap::from([(
                "localrealm".to_string(),
                vec!["admin".to_string()],
            )])),
            write_roles: None,
            plugins: None,
        });
        schema.insert("test_polygon_auth_admin".to_string(), auth_admin);

        let mut auth_optional = build_test_polygon_schema();
        auth_optional
            .topic
            .as_mut()
            .expect("test schema must have topic")
            .base = "polygon_auth_optional".to_string();
        auth_optional.auth = Some(StreamAuthConfig {
            required: false,
            read_roles: None,
            write_roles: None,
            plugins: None,
        });
        schema.insert("test_polygon_auth_optional".to_string(), auth_optional);

        // Explicit write_roles: only "producer" role from "localrealm" can write.
        let mut auth_write = build_test_polygon_schema();
        auth_write
            .topic
            .as_mut()
            .expect("test schema must have topic")
            .base = "polygon_auth_write".to_string();
        auth_write.auth = Some(StreamAuthConfig {
            required: true,
            read_roles: None,
            write_roles: Some(HashMap::from([(
                "localrealm".to_string(),
                vec!["producer".to_string()],
            )])),
            plugins: None,
        });
        schema.insert("test_polygon_auth_write".to_string(), auth_write);
    }
}

fn set_streaming_test_notification_schema(configuration: &mut Settings) {
    configuration.notification_schema = Some(HashMap::new());
    ensure_test_notification_schema(configuration, false);
}

fn set_streaming_auth_test_notification_schema(configuration: &mut Settings) {
    configuration.notification_schema = Some(HashMap::new());
    ensure_test_notification_schema(configuration, true);
}

fn set_jetstream_test_notification_schema(configuration: &mut Settings) {
    configuration.notification_schema = Some(HashMap::new());
    ensure_test_notification_schema(configuration, false);
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
        auth: AuthSettings::default(),
        metrics: MetricsSettings::default(),
        ecpds: None,
    }
}

#[cfg(feature = "ecmwf")]
fn start_sync_mock_ecpds_server() -> String {
    use std::io::{BufRead, BufReader, Write};

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let write_stream = match stream.try_clone() {
                Ok(s) => s,
                Err(_) => continue,
            };
            let mut reader = BufReader::new(stream);
            let mut request_line = String::new();
            if reader.read_line(&mut request_line).is_err() {
                continue;
            }

            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) if line.trim().is_empty() => break,
                    Err(_) => break,
                    _ => {}
                }
            }

            let (status, body) = if request_line.contains("id=ecpds-unavailable") {
                ("500 Internal Server Error", r#"{"error":"mock unavailable"}"#)
            } else if request_line.contains("id=ecpds-user") {
                ("200 OK", r#"{"destinationList":[{"name":"CIP","active":true},{"name":"FOO","active":true}],"success":"yes"}"#)
            } else {
                ("200 OK", r#"{"destinationList":[],"success":"yes"}"#)
            };

            let response = format!(
                "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                status,
                body.len(),
                body
            );
            let mut writer = write_stream;
            let _ = writer.write_all(response.as_bytes());
        }
    });

    format!("http://{}", addr)
}

#[cfg(feature = "ecmwf")]
fn ensure_ecpds_test_schemas(schema: &mut HashMap<String, EventSchema>) {
    let mut diss_ecpds = build_dissemination_schema();
    diss_ecpds
        .topic
        .as_mut()
        .expect("dissemination schema must have topic")
        .base = "diss_ecpds".to_string();
    diss_ecpds.auth = Some(StreamAuthConfig {
        required: true,
        read_roles: None,
        write_roles: None,
        plugins: Some(vec!["ecpds".to_string()]),
    });
    schema.insert("dissemination_ecpds".to_string(), diss_ecpds);
}

fn ensure_test_global_config_initialized() {
    TEST_GLOBAL_CONFIG.get_or_init(|| {
        let mut configuration = base_test_settings();
        ensure_test_notification_schema(&mut configuration, true);
        if let Some(schema) = configuration.notification_schema.as_mut() {
            apply_jetstream_test_polygon_js_policy(schema);
        }
        #[cfg(feature = "ecmwf")]
        {
            LazyLock::force(&MOCK_ECPDS_URL);
            configuration.ecpds = Some(aviso_ecmwf::config::EcpdsConfig {
                username: "masteruser".to_string(),
                password: "masterpass".to_string(),
                target_field: "name".to_string(),
                match_key: "destination".to_string(),
                cache_ttl_seconds: 300,
                servers: vec![MOCK_ECPDS_URL.clone()],
            });
            if let Some(schema) = configuration.notification_schema.as_mut() {
                ensure_ecpds_test_schemas(schema);
            }
        }
        Settings::init_global_config(&configuration);
    });
}

async fn spawn_server(
    mut configuration: Settings,
    auth_server_handle: Option<JoinHandle<std::io::Result<()>>>,
) -> RunningServer {
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
        _auth_server_handle: auth_server_handle,
    }
}

pub async fn spawn_app() -> TestApp {
    let running = DEFAULT_SERVER
        .get_or_init(|| async {
            let mut configuration = base_test_settings();
            ensure_test_notification_schema(&mut configuration, false);
            spawn_server(configuration, None).await
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

            spawn_server(configuration, None).await
        })
        .await;

    TestApp {
        address: running.address.clone(),
    }
}

pub async fn spawn_streaming_test_app_with_auth() -> TestApp {
    let running = STREAMING_AUTH_SERVER
        .get_or_init(|| async {
            let mut configuration = base_test_settings();
            set_streaming_auth_test_notification_schema(&mut configuration);
            let (auth_o_tron_url, auth_server_handle) = start_mock_auth_o_tron_server()
                .await
                .expect("mock auth-o-tron server must start");
            configuration.auth = AuthSettings {
                enabled: true,
                auth_o_tron_url,
                jwt_secret: "test-jwt-secret".to_string(),
                admin_roles: HashMap::from([("localrealm".to_string(), vec!["admin".to_string()])]),
                timeout_ms: 5_000,
                ..AuthSettings::default()
            };
            spawn_server(configuration, Some(auth_server_handle)).await
        })
        .await;

    TestApp {
        address: running.address.clone(),
    }
}

pub async fn spawn_streaming_test_app_with_trusted_proxy_auth() -> TestApp {
    let running = STREAMING_TRUSTED_PROXY_SERVER
        .get_or_init(|| async {
            let mut configuration = base_test_settings();
            set_streaming_auth_test_notification_schema(&mut configuration);
            configuration.auth = AuthSettings {
                enabled: true,
                mode: aviso_server::configuration::AuthMode::TrustedProxy,
                jwt_secret: "test-jwt-secret".to_string(),
                admin_roles: HashMap::from([("localrealm".to_string(), vec!["admin".to_string()])]),
                timeout_ms: 5_000,
                ..AuthSettings::default()
            };
            spawn_server(configuration, None).await
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
    let running = spawn_server(configuration, None).await;
    TestApp {
        address: running.address,
    }
}

async fn mock_authenticate(req: HttpRequest) -> HttpResponse {
    fn issue_mock_token(username: &str, roles: &[&str]) -> String {
        let claims = serde_json::json!({
            "sub": username,
            "username": username,
            "realm": "localrealm",
            "roles": roles,
            "attributes": {},
            "exp": (Utc::now().timestamp() + 3600) as usize,
            "iat": Utc::now().timestamp() as usize,
        });
        encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret("test-jwt-secret".as_bytes()),
        )
        .expect("mock token should encode")
    }

    let authorization = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());

    let token = match authorization {
        Some("Basic YWRtaW4tdXNlcjphZG1pbi1wYXNz") => {
            Some(issue_mock_token("admin-user", &["admin"]))
        }
        Some("Basic cmVhZGVyLXVzZXI6cmVhZGVyLXBhc3M=") => {
            Some(issue_mock_token("reader-user", &["reader"]))
        }
        Some(value) => {
            let mut parts = value.split_whitespace();
            match (parts.next(), parts.next(), parts.next()) {
                (Some(scheme), Some(candidate), None)
                    if scheme.eq_ignore_ascii_case("Bearer") && is_jwt_like(candidate) =>
                {
                    Some(candidate.to_string())
                }
                _ => None,
            }
        }
        _ => None,
    };

    match token {
        Some(token) => HttpResponse::Ok()
            .append_header((header::AUTHORIZATION, format!("Bearer {token}")))
            .body("Authenticated successfully"),
        None => HttpResponse::Unauthorized().finish(),
    }
}

async fn start_mock_auth_o_tron_server()
-> std::io::Result<(String, JoinHandle<std::io::Result<()>>)> {
    // Runtime-local ephemeral server to keep auth integration tests deterministic
    // and independent from external auth-o-tron availability.
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let server =
        HttpServer::new(|| App::new().route("/authenticate", web::get().to(mock_authenticate)))
            .disable_signals()
            .listen(listener)?
            .run();
    let server_handle = tokio::spawn(server);
    Ok((format!("http://{}", address), server_handle))
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
    let running = spawn_server(configuration, None).await;
    TestApp {
        address: running.address,
    }
}
