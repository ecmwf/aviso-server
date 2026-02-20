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
pub fn get_subscriber<Sink>(
    name: String,
    logging_config: Option<&LoggingSettings>,
    sink: Sink,
) -> impl Subscriber + Sync + Send
where
    Sink: for<'a> MakeWriter<'a> + Send + Sync + 'static,
{
    // Default to INFO when config is missing or unrecognized.
    let level = logging_config
        .map(|config| config.level.clone())
        .unwrap_or_else(|| "info".to_string());

    let level_filter = match level.to_lowercase().as_str() {
        "trace" => LevelFilter::TRACE,
        "debug" => LevelFilter::DEBUG,
        "warn" => LevelFilter::WARN,
        "error" => LevelFilter::ERROR,
        _ => LevelFilter::INFO,
    };
    let filter_layer = EnvFilter::default().add_directive(level_filter.into());

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
        // Keep library events raw: if a log does not set event_name/event_domain,
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
        json!({
            "timestamp": Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
            "severity_text": metadata.level().as_str(),
            "severity_number": OTelLogFormatter::severity_number(metadata.level()),
            "body": self.body,
            "resource": formatter.resource_json(),
            "attributes": self.attributes
        })
    }
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
        "event_domain" => "event.domain".to_string(),
        "service_name" => "service.name".to_string(),
        "service_version" => "service.version".to_string(),
        "error_type" => "error.type".to_string(),
        "error_message" => "error.message".to_string(),
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
    // Promote trace/span ids to canonical top-level correlation keys.
    if let Some(trace_id) = attributes.remove("otel.trace_id") {
        attributes.insert("trace_id".to_string(), trace_id);
    }
    if let Some(span_id) = attributes.remove("otel.span_id") {
        attributes.insert("span_id".to_string(), span_id);
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
        attrs.insert("trace_id".to_string(), json!("trace"));

        finalize_attributes(&mut attrs);

        assert!(!attrs.contains_key("service.name"));
        assert!(!attrs.contains_key("service.version"));
        assert_eq!(attrs.get("trace_id"), Some(&json!("trace")));
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
}
