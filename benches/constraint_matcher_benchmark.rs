use std::collections::HashMap;
use std::hint::black_box;
use std::sync::Once;

use aviso_server::configuration::{
    ApplicationSettings, AuthSettings, EventSchema, IdentifierFieldConfig,
    NotificationBackendSettings, PayloadConfig, Settings, TopicConfig, WatchEndpointSettings,
};
use aviso_server::notification::IdentifierConstraint;
use aviso_server::notification::wildcard_matcher::matches_notification_filters;
use aviso_validators::ValidationRules;
use aviso_validators::{EnumConstraint, NumericConstraint};
use criterion::{Criterion, criterion_group, criterion_main};

static INIT_SCHEMA: Once = Once::new();

fn build_empty_request() -> HashMap<String, String> {
    HashMap::new()
}

fn build_int_constraints() -> HashMap<String, IdentifierConstraint> {
    let mut constraints = HashMap::new();
    constraints.insert(
        "step".to_string(),
        IdentifierConstraint::Int(NumericConstraint::Gte(4)),
    );
    constraints
}

fn build_float_constraints() -> HashMap<String, IdentifierConstraint> {
    let mut constraints = HashMap::new();
    constraints.insert(
        "anomaly".to_string(),
        IdentifierConstraint::Float(NumericConstraint::Between(40.0, 50.0)),
    );
    constraints
}

fn build_mixed_constraints() -> HashMap<String, IdentifierConstraint> {
    let mut constraints = HashMap::new();
    constraints.insert(
        "region".to_string(),
        IdentifierConstraint::Enum(EnumConstraint::In(vec![
            "north".to_string(),
            "south".to_string(),
        ])),
    );
    constraints.insert(
        "severity".to_string(),
        IdentifierConstraint::Int(NumericConstraint::Gte(4)),
    );
    constraints.insert(
        "anomaly".to_string(),
        IdentifierConstraint::Float(NumericConstraint::Gt(40.0)),
    );
    constraints
}

fn init_benchmark_schema() {
    INIT_SCHEMA.call_once(|| {
        // Keep schema setup minimal so benchmark time reflects matcher work,
        // not configuration complexity.
        let mut notification_schema = HashMap::new();

        let mut mars_identifier = HashMap::new();
        mars_identifier.insert(
            "class".to_string(),
            IdentifierFieldConfig::with_rule(ValidationRules::StringHandler {
                max_length: Some(2),
                required: true,
            }),
        );
        mars_identifier.insert(
            "expver".to_string(),
            IdentifierFieldConfig::with_rule(ValidationRules::ExpverHandler {
                default: Some("0001".to_string()),
                required: false,
            }),
        );
        mars_identifier.insert(
            "domain".to_string(),
            IdentifierFieldConfig::with_rule(ValidationRules::EnumHandler {
                values: vec!["g".to_string(), "a".to_string(), "z".to_string()],
                required: false,
            }),
        );
        mars_identifier.insert(
            "date".to_string(),
            IdentifierFieldConfig::with_rule(ValidationRules::DateHandler {
                canonical_format: "%Y%m%d".to_string(),
                required: false,
            }),
        );
        mars_identifier.insert(
            "time".to_string(),
            IdentifierFieldConfig::with_rule(ValidationRules::TimeHandler { required: false }),
        );
        mars_identifier.insert(
            "stream".to_string(),
            IdentifierFieldConfig::with_rule(ValidationRules::StringHandler {
                max_length: None,
                required: false,
            }),
        );
        mars_identifier.insert(
            "step".to_string(),
            IdentifierFieldConfig::with_rule(ValidationRules::IntHandler {
                range: Some([0, 100000]),
                required: false,
            }),
        );
        notification_schema.insert(
            "mars".to_string(),
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
                identifier: mars_identifier,
                storage_policy: None,
                auth: None,
            },
        );

        let mut extreme_identifier = HashMap::new();
        extreme_identifier.insert(
            "region".to_string(),
            IdentifierFieldConfig::with_rule(ValidationRules::EnumHandler {
                values: vec!["north".to_string(), "south".to_string()],
                required: false,
            }),
        );
        extreme_identifier.insert(
            "run_time".to_string(),
            IdentifierFieldConfig::with_rule(ValidationRules::TimeHandler { required: false }),
        );
        extreme_identifier.insert(
            "severity".to_string(),
            IdentifierFieldConfig::with_rule(ValidationRules::IntHandler {
                range: Some([1, 7]),
                required: false,
            }),
        );
        extreme_identifier.insert(
            "anomaly".to_string(),
            IdentifierFieldConfig::with_rule(ValidationRules::FloatHandler {
                range: Some([0.0, 200.0]),
                required: false,
            }),
        );
        notification_schema.insert(
            "extreme".to_string(),
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
                identifier: extreme_identifier,
                storage_policy: None,
                auth: None,
            },
        );

        let settings = Settings {
            application: ApplicationSettings {
                host: "127.0.0.1".to_string(),
                port: 0,
                base_url: "localhost:8000".to_string(),
                static_files_path: "./src/static".to_string(),
            },
            notification_backend: NotificationBackendSettings {
                kind: "in_memory".to_string(),
                in_memory: None,
                jetstream: None,
            },
            logging: None,
            notification_schema: Some(notification_schema),
            watch_endpoint: WatchEndpointSettings::default(),
            auth: AuthSettings::default(),
        };

        settings.init_global_config();
    });
}

fn benchmark_constraint_matcher(c: &mut Criterion) {
    init_benchmark_schema();

    let request = build_empty_request();
    let payload = "";
    let metadata = None;

    // Mars key_order in tests/config: class, expver, domain, date, time, stream, step
    let mars_topic = "mars.od.0001.g.20250706.1200.enfo.5";
    // Extreme key_order in tests/config: region, run_time, severity, anomaly
    // Note: decimal token uses wire encoding ("42%2E5").
    let extreme_topic = "extreme.north.1200.4.42%2E5";

    let int_constraints = build_int_constraints();
    let float_constraints = build_float_constraints();
    let mixed_constraints = build_mixed_constraints();
    let no_constraints: HashMap<String, IdentifierConstraint> = HashMap::new();

    // Baseline matcher cost when no identifier constraints are active.
    c.bench_function("constraint_matcher/no_constraints", |b| {
        b.iter(|| {
            black_box(matches_notification_filters(
                black_box(mars_topic),
                black_box(&request),
                black_box(&no_constraints),
                black_box(metadata),
                black_box(payload),
            ))
        });
    });

    // Single numeric constraint on an integer identifier field.
    c.bench_function("constraint_matcher/int_single", |b| {
        b.iter(|| {
            black_box(matches_notification_filters(
                black_box(mars_topic),
                black_box(&request),
                black_box(&int_constraints),
                black_box(metadata),
                black_box(payload),
            ))
        });
    });

    // Single numeric constraint on a floating-point identifier field.
    c.bench_function("constraint_matcher/float_single", |b| {
        b.iter(|| {
            black_box(matches_notification_filters(
                black_box(extreme_topic),
                black_box(&request),
                black_box(&float_constraints),
                black_box(metadata),
                black_box(payload),
            ))
        });
    });

    // Mixed profile exercising enum + int + float constraints together.
    c.bench_function("constraint_matcher/mixed_three", |b| {
        b.iter(|| {
            black_box(matches_notification_filters(
                black_box(extreme_topic),
                black_box(&request),
                black_box(&mixed_constraints),
                black_box(metadata),
                black_box(payload),
            ))
        });
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(150);
    targets = benchmark_constraint_matcher
}
criterion_main!(benches);
