// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use crate::configuration::LoggingSettings;
use chrono::{SecondsFormat, Utc};
use regex::Regex;
use serde_json::{Map, Value, json};
use std::fmt;
use tracing::field::{Field, Visit};
use tracing::level_filters::LevelFilter;
use tracing::subscriber::set_global_default;
use tracing::{Event, Level, Subscriber};
use tracing_log::LogTracer;
use tracing_subscriber::fmt::FormattedFields;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::fmt::format::{FormatEvent, FormatFields, Writer};
use tracing_subscriber::fmt::{FmtContext, MakeWriter};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{EnvFilter, Registry, layer::SubscriberExt};

pub const SERVICE_NAME: &str = env!("CARGO_PKG_NAME");
pub const SERVICE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Build a tracing subscriber that emits OTel-aligned JSON logs.
///
/// Filter resolution order:
/// 1. If the `RUST_LOG` env var is set, it wins outright. Operators get full
///    `EnvFilter` directive syntax (e.g. `info,aviso_server=debug,actix_web=warn`)
///    for runtime triage without a code change. A malformed `RUST_LOG` is
///    logged to stderr and the configured default is used instead.
/// 2. Otherwise, build a default filter from `logging.level` (or `info` when
///    no config is supplied) and add a small set of mute directives for
///    framework internals that are routinely chatty at info
///    (see [`default_mute_directives`]).
pub fn get_subscriber<Sink>(
    name: String,
    logging_config: Option<&LoggingSettings>,
    sink: Sink,
) -> impl Subscriber + Sync + Send
where
    Sink: for<'a> MakeWriter<'a> + Send + Sync + 'static,
{
    let level = logging_config
        .map(|config| config.level.clone())
        .unwrap_or_else(|| "info".to_string());

    let filter_layer = build_env_filter(&level);

    let formatter = OTelLogFormatter::new(name);
    let formatting_layer = tracing_subscriber::fmt::layer()
        .with_writer(sink)
        .with_ansi(false)
        .with_span_events(FmtSpan::NONE)
        .event_format(formatter);

    Registry::default()
        .with(filter_layer)
        .with(formatting_layer)
}

/// Build the runtime `EnvFilter` honouring `RUST_LOG` first, then falling back
/// to the configured base level plus the default mute directives.
///
/// Kept as a free function so unit tests can exercise the resolution rules
/// without standing up the full subscriber.
fn build_env_filter(default_level: &str) -> EnvFilter {
    if let Some(raw) = std::env::var_os("RUST_LOG")
        && let Some(filter) = parse_rust_log_value(&raw.to_string_lossy(), default_level)
    {
        return filter;
    }

    apply_default_mute_directives(
        EnvFilter::default().add_directive(parse_level_filter(default_level).into()),
    )
}

/// Parse an explicit `RUST_LOG` value. Returns `None` to signal "fall back to
/// the default filter" rather than installing the value as-is.
///
/// Two values must be treated as "fall back":
/// 1. Empty or whitespace-only strings. `EnvFilter::try_new("")` succeeds and
///    yields a filter that matches nothing, which would silently silence the
///    entire process. Some deployment systems (Kubernetes downward API,
///    docker-compose `${VAR:-}`) export unset variables as empty strings, so
///    this case is reachable in production.
/// 2. Strings that fail `EnvFilter` parsing. The error is surfaced to stderr
///    (the subscriber is not yet installed, so `tracing::*!` would be lost)
///    and the operator gets a real malformed example to compare against.
fn parse_rust_log_value(value: &str, default_level: &str) -> Option<EnvFilter> {
    if value.trim().is_empty() {
        return None;
    }
    match EnvFilter::try_new(value) {
        Ok(filter) => Some(filter),
        Err(error) => {
            eprintln!(
                "warning: RUST_LOG is set but could not be parsed as an EnvFilter \
                 directive ({error}); falling back to logging.level={default_level}. \
                 Example valid value: RUST_LOG=info,aviso_server=debug,actix_web=warn"
            );
            None
        }
    }
}

fn parse_level_filter(level: &str) -> LevelFilter {
    match level.to_lowercase().as_str() {
        "trace" => LevelFilter::TRACE,
        "debug" => LevelFilter::DEBUG,
        "warn" => LevelFilter::WARN,
        "error" => LevelFilter::ERROR,
        _ => LevelFilter::INFO,
    }
}

/// Default per-target mute directives applied when `RUST_LOG` is unset.
///
/// Each directive narrows a noisy dependency below the application-wide
/// default level. The list is intentionally short; anything beyond
/// "framework internals that flood at info" should be opt-in via `RUST_LOG`.
fn default_mute_directives() -> &'static [&'static str] {
    &[
        // Actix request/server lifecycle (worker started, accepting, etc.).
        "actix_web=warn",
        "actix_server=warn",
        // NATS keeps connect/reconnect at info and message-level chatter at
        // debug; info is the right floor for an operational surface.
        "async_nats=info",
    ]
}

fn apply_default_mute_directives(mut filter: EnvFilter) -> EnvFilter {
    for directive_str in default_mute_directives() {
        match directive_str.parse() {
            Ok(directive) => filter = filter.add_directive(directive),
            Err(error) => {
                // A bad hardcoded directive is a developer error, not an
                // operator one; log to stderr so it does not silently drift.
                eprintln!(
                    "warning: failed to parse default mute directive {directive_str:?} \
                     ({error}); skipping"
                );
            }
        }
    }
    filter
}

/// Register tracing globally and bridge `log` records into tracing.
pub fn init_subscriber(subscriber: impl Subscriber + Sync + Send) {
    LogTracer::init().expect("Failed to set logger");
    set_global_default(subscriber).expect("Failed to set subscriber");
}

struct OTelLogFormatter {
    service_name: String,
    service_version: String,
    k8s_namespace: Option<String>,
    k8s_pod_name: Option<String>,
}

impl OTelLogFormatter {
    fn new(service_name: String) -> Self {
        Self {
            service_name,
            service_version: SERVICE_VERSION.to_string(),
            k8s_namespace: std::env::var("K8S_NAMESPACE_NAME").ok(),
            k8s_pod_name: std::env::var("K8S_POD_NAME")
                .ok()
                .or_else(|| std::env::var("HOSTNAME").ok()),
        }
    }

    fn severity_number(level: &Level) -> u8 {
        // OpenTelemetry severity buckets: TRACE(1), DEBUG(5), INFO(9), WARN(13), ERROR(17).
        match *level {
            Level::TRACE => 1,
            Level::DEBUG => 5,
            Level::INFO => 9,
            Level::WARN => 13,
            Level::ERROR => 17,
        }
    }

    fn resource_json(&self) -> Map<String, Value> {
        // Resource attributes describe the process/runtime identity, not per-event data.
        let mut resource = Map::new();
        resource.insert("service.name".to_string(), json!(self.service_name));
        resource.insert("service.version".to_string(), json!(self.service_version));
        if let Some(namespace) = &self.k8s_namespace {
            resource.insert("k8s.namespace.name".to_string(), json!(namespace));
        }
        if let Some(pod_name) = &self.k8s_pod_name {
            resource.insert("k8s.pod.name".to_string(), json!(pod_name));
        }
        resource
    }
}

impl<S, N> FormatEvent<S, N> for OTelLogFormatter
where
    S: Subscriber + for<'span> LookupSpan<'span>,
    N: for<'writer> FormatFields<'writer> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let metadata = event.metadata();
        let mut visitor = JsonVisitor::default();
        event.record(&mut visitor);
        let record = LogRecordBuilder::from_event(metadata, visitor.fields)
            .with_request_id_from_span(ctx)
            .with_code_location(metadata)
            .finalize()
            .build_json(self, metadata);

        let serialized = serde_json::to_string(&record).map_err(|_| fmt::Error)?;
        writer.write_str(&serialized)?;
        writer.write_str("\n")
    }
}

struct LogRecordBuilder {
    body: String,
    attributes: Map<String, Value>,
}

impl LogRecordBuilder {
    fn from_event(metadata: &tracing::Metadata<'_>, mut fields: Map<String, Value>) -> Self {
        let body = fields
            .remove("message")
            .and_then(|v| v.as_str().map(ToString::to_string))
            .unwrap_or_else(|| metadata.name().to_string());

        Self {
            body: redact_message(&body),
            attributes: normalize_and_redact_attributes(fields),
        }
    }

    fn with_request_id_from_span<S, N>(mut self, ctx: &FmtContext<'_, S, N>) -> Self
    where
        S: Subscriber + for<'span> LookupSpan<'span>,
        N: for<'writer> FormatFields<'writer> + 'static,
    {
        // Keep library events raw: if a log does not set event_name,
        // we do not synthesize fallback values.
        populate_request_id_from_span(ctx, &mut self.attributes);
        self
    }

    fn with_code_location(mut self, metadata: &tracing::Metadata<'_>) -> Self {
        add_code_location_attributes(&mut self.attributes, metadata);
        self
    }

    fn finalize(mut self) -> Self {
        finalize_attributes(&mut self.attributes);
        self
    }

    fn build_json(self, formatter: &OTelLogFormatter, metadata: &tracing::Metadata<'_>) -> Value {
        assemble_log_json(
            Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
            metadata.level().as_str(),
            OTelLogFormatter::severity_number(metadata.level()),
            self.body,
            formatter.resource_json(),
            self.attributes,
        )
    }
}

/// Assemble the final OTel log record JSON from pre-computed parts.
///
/// `traceId` and `spanId` are promoted from `attributes` to top-level fields
/// per the OTel log data model; they are absent when no trace context was set.
fn assemble_log_json(
    timestamp: String,
    severity_text: &str,
    severity_number: u8,
    body: String,
    resource: Map<String, Value>,
    mut attributes: Map<String, Value>,
) -> Value {
    let trace_id = attributes.remove("traceId");
    let span_id = attributes.remove("spanId");

    let mut record = json!({
        "timestamp": timestamp,
        "severityText": severity_text,
        "severityNumber": severity_number,
        "body": body,
        "resource": resource,
        "attributes": attributes,
    });

    if let Some(id) = trace_id {
        record["traceId"] = id;
    }
    if let Some(id) = span_id {
        record["spanId"] = id;
    }

    record
}

#[derive(Default)]
struct JsonVisitor {
    fields: Map<String, Value>,
}

impl JsonVisitor {
    fn insert(&mut self, field: &Field, value: Value) {
        self.fields.insert(field.name().to_string(), value);
    }
}

impl Visit for JsonVisitor {
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.insert(field, json!(value));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.insert(field, json!(value));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.insert(field, json!(value));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.insert(field, json!(value));
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.insert(field, json!(value));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.insert(field, json!(format!("{:?}", value)));
    }
}

fn normalize_attribute_key(raw: &str) -> String {
    // Normalize selected callsite keys to the canonical attribute names we expose.
    match raw {
        "event_name" => "event.name".to_string(),
        "service_name" => "service.name".to_string(),
        "service_version" => "service.version".to_string(),
        "error_type" => "error.type".to_string(),
        "error_message" => "exception.message".to_string(),
        _ => raw.to_string(),
    }
}

fn normalize_and_redact_attributes(fields: Map<String, Value>) -> Map<String, Value> {
    fields
        .into_iter()
        .map(|(key, value)| {
            (
                normalize_attribute_key(&key),
                redact_json_value(&key, value),
            )
        })
        .collect()
}

fn add_code_location_attributes(
    attributes: &mut Map<String, Value>,
    metadata: &tracing::Metadata<'_>,
) {
    attributes
        .entry("code.target".to_string())
        .or_insert_with(|| json!(metadata.target()));
    if let Some(file) = metadata.file() {
        attributes.insert("code.filepath".to_string(), json!(file));
    }
    if let Some(line) = metadata.line() {
        attributes.insert("code.lineno".to_string(), json!(line));
    }
}

fn finalize_attributes(attributes: &mut Map<String, Value>) {
    promote_trace_correlation(attributes);
    // Keep identity fields in `resource` only to avoid duplicate dimensions.
    attributes.remove("service.name");
    attributes.remove("service.version");
    attributes.remove("k8s.namespace.name");
    attributes.remove("k8s.pod.name");
}

fn promote_trace_correlation(attributes: &mut Map<String, Value>) {
    // Rename OTel span extension keys to camelCase LogRecord field names so that
    // build_json() can lift them to the top level.
    if let Some(trace_id) = attributes.remove("otel.trace_id") {
        attributes.insert("traceId".to_string(), trace_id);
    }
    if let Some(span_id) = attributes.remove("otel.span_id") {
        attributes.insert("spanId".to_string(), span_id);
    }
}

fn populate_request_id_from_span<S, N>(
    ctx: &FmtContext<'_, S, N>,
    attributes: &mut Map<String, Value>,
) where
    S: Subscriber + for<'span> LookupSpan<'span>,
    N: for<'writer> FormatFields<'writer> + 'static,
{
    // Event fields win over span-derived values.
    if attributes.contains_key("request_id") {
        return;
    }

    // No current span means no request context to hydrate from.
    let Some(span) = ctx.lookup_current() else {
        return;
    };

    // Prefer the innermost span first because it is the most request-specific context.
    let scoped_spans = span.scope().from_root().collect::<Vec<_>>();
    for scope_span in scoped_spans.into_iter().rev() {
        let extensions = scope_span.extensions();
        let Some(formatted) = extensions.get::<FormattedFields<N>>() else {
            continue;
        };
        if let Some(request_id) = extract_request_id_from_formatted_fields(formatted) {
            attributes.insert("request_id".to_string(), json!(request_id));
            return;
        }
    }
}

fn extract_request_id_from_formatted_fields<N>(
    formatted_fields: &FormattedFields<N>,
) -> Option<String> {
    // `FormattedFields` is key=value text; this extracts request_id=<value>.
    extract_request_id_from_text(formatted_fields.fields.as_str())
}

fn extract_request_id_from_text(formatted_fields: &str) -> Option<String> {
    static REQUEST_ID_RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let request_id_re = REQUEST_ID_RE.get_or_init(|| {
        Regex::new(r#"request_id=("[^"]+"|[^ ]+)"#).expect("valid request_id regex")
    });

    let captures = request_id_re.captures(formatted_fields)?;
    let raw = captures.get(1)?.as_str();
    Some(raw.trim_matches('"').to_string())
}

fn redact_json_value(key: &str, value: Value) -> Value {
    // Key-based redaction handles fields that should never leave the process.
    if is_sensitive_key(key) {
        return json!("[REDACTED]");
    }

    match value {
        Value::String(s) => {
            if key == "config"
                && let Ok(parsed) = serde_json::from_str::<Value>(&s)
            {
                return redact_embedded_json(parsed);
            }
            if let Some(redacted_url) = redact_url_userinfo(&s) {
                return Value::String(redacted_url);
            }
            Value::String(redact_message(&s))
        }
        other => other,
    }
}

pub fn is_sensitive_key(key: &str) -> bool {
    let lower_key = key.to_ascii_lowercase();
    lower_key.contains("password")
        || lower_key.contains("secret")
        || lower_key.contains("token")
        || lower_key.contains("authorization")
        || lower_key.contains("api_key")
}

fn redact_embedded_json(value: Value) -> Value {
    match value {
        Value::Object(obj) => {
            let mut redacted = Map::new();
            for (key, child) in obj {
                if is_sensitive_key(&key) {
                    redacted.insert(key, json!("[REDACTED]"));
                } else {
                    redacted.insert(key, redact_embedded_json(child));
                }
            }
            Value::Object(redacted)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(redact_embedded_json).collect()),
        Value::String(s) => {
            if let Some(redacted_url) = redact_url_userinfo(&s) {
                Value::String(redacted_url)
            } else {
                Value::String(redact_message(&s))
            }
        }
        other => other,
    }
}

pub fn redact_url_userinfo(raw: &str) -> Option<String> {
    let scheme_pos = raw.find("://")?;
    let authority_start = scheme_pos + 3;
    let authority_end_rel = raw[authority_start..]
        .find(['/', '?', '#'])
        .unwrap_or(raw.len() - authority_start);
    let authority_end = authority_start + authority_end_rel;
    let authority = &raw[authority_start..authority_end];
    let at_pos = authority.find('@')?;

    let mut redacted = String::with_capacity(raw.len());
    redacted.push_str(&raw[..authority_start]);
    redacted.push_str("[REDACTED]@");
    redacted.push_str(&authority[at_pos + 1..]);
    redacted.push_str(&raw[authority_end..]);
    Some(redacted)
}

fn redact_message(message: &str) -> String {
    // Message-level redaction is defensive for free-form strings.
    static PASSWORD_RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    static TOKEN_RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    static BEARER_RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();

    let password_re = PASSWORD_RE
        .get_or_init(|| Regex::new(r"(?i)(password\s*[=:]\s*)\S+").expect("valid regex"));
    let token_re =
        TOKEN_RE.get_or_init(|| Regex::new(r"(?i)(token\s*[=:]\s*)\S+").expect("valid regex"));
    let bearer_re = BEARER_RE
        .get_or_init(|| Regex::new(r"(?i)(bearer\s+)[a-z0-9\._\-]+").expect("valid regex"));

    let redacted = password_re.replace_all(message, "$1[REDACTED]");
    let redacted = token_re.replace_all(&redacted, "$1[REDACTED]");
    bearer_re
        .replace_all(&redacted, "$1[REDACTED]")
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_id_extractor_handles_quoted_and_unquoted_values() {
        assert_eq!(
            extract_request_id_from_text(r#"request_id="abc-123" foo=bar"#),
            Some("abc-123".to_string())
        );
        assert_eq!(
            extract_request_id_from_text("request_id=req-42 foo=bar"),
            Some("req-42".to_string())
        );
        assert_eq!(extract_request_id_from_text("foo=bar"), None);
    }

    #[test]
    fn normalize_and_redact_redacts_sensitive_keys() {
        let mut fields = Map::new();
        fields.insert("api_key".to_string(), json!("secret-value"));
        fields.insert("event_name".to_string(), json!("api.test.succeeded"));
        let attrs = normalize_and_redact_attributes(fields);

        assert_eq!(attrs.get("api_key"), Some(&json!("[REDACTED]")));
        assert_eq!(attrs.get("event.name"), Some(&json!("api.test.succeeded")));
    }

    #[test]
    fn finalize_attributes_removes_identity_duplicates() {
        let mut attrs = Map::new();
        attrs.insert("service.name".to_string(), json!("aviso-server"));
        attrs.insert("service.version".to_string(), json!("0.1.3"));
        attrs.insert("otel.trace_id".to_string(), json!("abc123"));

        finalize_attributes(&mut attrs);

        assert!(!attrs.contains_key("service.name"));
        assert!(!attrs.contains_key("service.version"));
        // otel.trace_id is promoted to traceId for top-level placement in build_json.
        assert!(!attrs.contains_key("otel.trace_id"));
        assert_eq!(attrs.get("traceId"), Some(&json!("abc123")));
    }

    #[test]
    fn redact_url_userinfo_redacts_credentials_in_authority() {
        let redacted =
            redact_url_userinfo("nats://user:pass@localhost:4222").expect("must redact userinfo");
        assert_eq!(redacted, "nats://[REDACTED]@localhost:4222");
        assert_eq!(redact_url_userinfo("nats://localhost:4222"), None);
    }

    #[test]
    fn config_redaction_parses_json_and_sanitizes_url_userinfo() {
        let config_json = r#"{"nats_url":"nats://user:pass@localhost:4222","token":"secret"}"#;
        let value = redact_json_value("config", Value::String(config_json.to_string()));
        assert_eq!(
            value.get("nats_url"),
            Some(&json!("nats://[REDACTED]@localhost:4222"))
        );
        assert_eq!(value.get("token"), Some(&json!("[REDACTED]")));
    }

    #[test]
    fn assemble_log_json_produces_otel_compliant_schema() {
        let mut attributes = Map::new();
        attributes.insert("event.name".to_string(), json!("api.request.succeeded"));
        attributes.insert("traceId".to_string(), json!("abc123"));
        attributes.insert("spanId".to_string(), json!("def456"));

        let resource = Map::new();
        let record = assemble_log_json(
            "2024-01-01T00:00:00Z".to_string(),
            "INFO",
            9,
            "request handled".to_string(),
            resource,
            attributes,
        );

        // OTel top-level LogRecord fields must be at the root.
        assert_eq!(record["severityText"], "INFO");
        assert_eq!(record["severityNumber"], 9);
        assert_eq!(record["traceId"], "abc123");
        assert_eq!(record["spanId"], "def456");

        // Trace fields must not be duplicated inside attributes.
        let attrs = record["attributes"]
            .as_object()
            .expect("attributes is object");
        assert!(!attrs.contains_key("traceId"));
        assert!(!attrs.contains_key("spanId"));
    }

    #[test]
    fn assemble_log_json_omits_trace_fields_when_absent() {
        let record = assemble_log_json(
            "2024-01-01T00:00:00Z".to_string(),
            "WARN",
            13,
            "no trace context".to_string(),
            Map::new(),
            Map::new(),
        );

        assert!(record.get("traceId").is_none());
        assert!(record.get("spanId").is_none());
    }

    #[test]
    fn parse_level_filter_maps_known_levels_and_falls_back_to_info() {
        assert_eq!(parse_level_filter("trace"), LevelFilter::TRACE);
        assert_eq!(parse_level_filter("DEBUG"), LevelFilter::DEBUG);
        assert_eq!(parse_level_filter("warn"), LevelFilter::WARN);
        assert_eq!(parse_level_filter("error"), LevelFilter::ERROR);
        assert_eq!(parse_level_filter("info"), LevelFilter::INFO);
        // Typos and empty strings must not panic; INFO is the safe floor.
        assert_eq!(parse_level_filter("nonsense"), LevelFilter::INFO);
        assert_eq!(parse_level_filter(""), LevelFilter::INFO);
    }

    #[test]
    fn default_mute_directives_all_parse_as_valid_envfilter_directives() {
        // A typo in this list would silently disappear at runtime (the
        // skip-and-eprintln branch in apply_default_mute_directives), so
        // we pin the contract here at compile/test time instead.
        for directive in default_mute_directives() {
            directive
                .parse::<tracing_subscriber::filter::Directive>()
                .unwrap_or_else(|error| {
                    panic!("default mute directive {directive:?} must parse: {error}")
                });
        }
    }

    #[test]
    fn parse_rust_log_value_treats_empty_or_whitespace_as_unset() {
        // Critical regression test. EnvFilter::try_new("") silently succeeds
        // with a filter that matches nothing, which would black-hole the
        // entire process. Empty/whitespace must fall through to the default.
        // Some deployment systems (Kubernetes downward API,
        // docker-compose ${VAR:-}) export unset env vars as empty strings,
        // so this case is reachable in production, not just a theoretical
        // edge case.
        assert!(parse_rust_log_value("", "info").is_none());
        assert!(parse_rust_log_value(" ", "info").is_none());
        assert!(parse_rust_log_value("   ", "info").is_none());
        assert!(parse_rust_log_value("\t", "info").is_none());
        assert!(parse_rust_log_value("\n", "info").is_none());
        assert!(parse_rust_log_value("\t\n  ", "info").is_none());
    }

    #[test]
    fn parse_rust_log_value_accepts_valid_directives() {
        assert!(parse_rust_log_value("info", "warn").is_some());
        assert!(parse_rust_log_value("info,aviso_server=debug", "warn").is_some());
        assert!(parse_rust_log_value("warn,aviso_server::auth=trace", "info").is_some());
        assert!(parse_rust_log_value("debug", "info").is_some());
    }

    #[test]
    fn parse_rust_log_value_falls_back_on_unparseable_input() {
        // Confirmed parse failures via EnvFilter::try_new (tracing-subscriber
        // 0.3.x): an empty target before `=` (e.g. "=warn") and a non-level
        // value after `=` ("foo=BOGUSLEVEL"). Note that
        // "info aviso_server=debug" (a missing comma) is NOT a parse error;
        // it parses as a single target name with a space, so we cannot use
        // it as a fallback test case.
        assert!(parse_rust_log_value("=warn", "info").is_none());
        assert!(parse_rust_log_value("info,foo=BOGUSLEVEL", "info").is_none());
    }

    /// Find the macro level (`trace`/`debug`/`info`/`warn`/`error`) of the
    /// `tracing::*!` call whose argument list references a given `event_name`.
    ///
    /// Used by the level-pinning tests below. Source-scan rather than runtime
    /// capture because (a) most demoted call sites require network/database
    /// setup to exercise end-to-end and (b) a regression in the macro choice
    /// (e.g. `tracing::debug!` flipped back to `tracing::info!`) is a textual
    /// change, so a textual pin is the minimum-overhead detection mechanism.
    ///
    /// Two source patterns are recognised:
    /// 1. Literal in the macro arguments: `event_name = "value"`. The macro
    ///    opens BEFORE the literal in the file. Scanned backwards.
    /// 2. Literal as the value of a `let event_name = ...` binding, with
    ///    `event_name = event_name` (or shorthand `event_name`) inside the
    ///    macro that follows. The macro opens AFTER the literal. Scanned
    ///    forwards. This is how `notification_backend::jetstream::publisher`
    ///    handles the headers/no-headers conditional event name.
    fn macro_level_for_event_name(src: &str, event_name: &str) -> &'static str {
        let macro_re = Regex::new(r"\b(trace|debug|info|warn|error)!\s*\(")
            .expect("level-pinning macro regex must compile");

        let in_macro_needle = format!("event_name = \"{event_name}\"");
        if let Some(idx) = src.find(&in_macro_needle) {
            let last = macro_re.find_iter(&src[..idx]).last().unwrap_or_else(|| {
                panic!(
                    "no tracing macro call found before event_name {event_name:?} \
                     — call site refactored?"
                )
            });
            return capture_level(&macro_re, last.as_str());
        }

        let quoted_needle = format!("\"{event_name}\"");
        if let Some(idx) = src.find(&quoted_needle) {
            let next = macro_re.find(&src[idx..]).unwrap_or_else(|| {
                panic!(
                    "literal {event_name:?} found but no following tracing macro \
                     — call site refactored?"
                )
            });
            return capture_level(&macro_re, next.as_str());
        }

        panic!("event_name {event_name:?} not found in source");
    }

    fn capture_level(macro_re: &Regex, matched_text: &str) -> &'static str {
        let captures = macro_re
            .captures(matched_text)
            .expect("macro regex must capture on its own match");
        match captures.get(1).expect("level capture group").as_str() {
            "trace" => "trace",
            "debug" => "debug",
            "info" => "info",
            "warn" => "warn",
            "error" => "error",
            other => panic!("unexpected macro level {other:?}"),
        }
    }

    #[test]
    fn demoted_events_pin_their_log_level_in_source() {
        // Regression-prevention pins for the events whose level was
        // explicitly chosen by PR #86's volume reduction. Without these
        // pins, a textual flip back to `tracing::info!` would silently
        // undo Phase 2 / Phase 3 of the volume reduction and the rest of
        // the test suite would still pass. The pin is a source-text scan
        // because the alternative — installing a capturing tracing
        // subscriber and exercising every call site end-to-end — would
        // require network and backend mocks well beyond the scope of a
        // unit test, and would still not catch the textual regression
        // class this test is designed to catch.
        let storage_src = include_str!("handlers/storage.rs");
        let publisher_src = include_str!("notification_backend/jetstream/publisher.rs");
        let streaming_src = include_str!("routes/streaming.rs");
        let notify_src = include_str!("routes/notify.rs");

        // Phase 2: notification chain demotions to debug.
        assert_eq!(
            macro_level_for_event_name(storage_src, "notification.storage.spatial.succeeded"),
            "debug",
        );
        assert_eq!(
            macro_level_for_event_name(storage_src, "notification.storage.succeeded"),
            "debug",
        );
        assert_eq!(
            macro_level_for_event_name(publisher_src, "backend.jetstream.publish.succeeded"),
            "debug",
        );
        assert_eq!(
            macro_level_for_event_name(
                publisher_src,
                "backend.jetstream.publish_with_headers.succeeded"
            ),
            "debug",
        );

        // Phase 3 safe half: ECPDS demotion to debug.
        assert_eq!(
            macro_level_for_event_name(streaming_src, "auth.ecpds.admin.bypass"),
            "debug",
        );

        // Kept-info pins. If these flip to debug the audit/observability
        // contract changes; updating the level here without also updating
        // the corresponding runbook entry (docs/src/ecpds-runbook.md for
        // check.allowed; PR #86 description for the canonical
        // notification line) is a contract regression even if the gates
        // pass.
        assert_eq!(
            macro_level_for_event_name(notify_src, "api.notification.processed"),
            "info",
        );
        assert_eq!(
            macro_level_for_event_name(streaming_src, "auth.ecpds.check.allowed"),
            "info",
        );
    }
}
