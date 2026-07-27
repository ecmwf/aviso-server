// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Push-based OTLP log export.
//!
//! Bridges `tracing` events into the OpenTelemetry logs pipeline and ships
//! them to a collector over gRPC or HTTP. The stdout JSON pipeline in the
//! parent module is unaffected: OTLP export is an additional, config-gated
//! sink, and export runs on a background batch thread so a slow or absent
//! collector never blocks request handling.

use crate::configuration::{OtlpProtocol, OtlpSettings};
use opentelemetry::logs::{AnyValue, LogRecord, Severity};
use opentelemetry::{InstrumentationScope, KeyValue};
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::logs::{
    BatchLogProcessor, LogBatch, LogExporter, LogProcessor, SdkLogRecord, SdkLoggerProvider,
};
use prometheus::{IntCounter, Registry};
use std::sync::LazyLock;
use std::time::Duration;
use tracing::Subscriber;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{EnvFilter, Layer};

/// Records withheld from OTLP export by the redaction guard.
///
/// Static rather than part of `AppMetrics`: the log pipeline is a
/// process-wide singleton installed in `main` before the application (and
/// its metrics registry) is built. [`register_otlp_metrics`] links the
/// counters into the served registry; increments before registration are
/// preserved because Prometheus collectors keep state independently of
/// registries.
static OTLP_SUPPRESSED_LOG_RECORDS: LazyLock<IntCounter> = LazyLock::new(|| {
    IntCounter::new(
        "aviso_otlp_suppressed_log_records_total",
        "Log records withheld from OTLP export by the redaction guard \
         (sensitive attribute key or credentialed URL value). The stdout \
         copy of each suppressed record is still emitted with field-level \
         redaction. A non-zero rate means OpenSearch is missing records \
         that stdout has; check for new call sites logging sensitive \
         field names.",
    )
    .expect("metric definition must be valid")
});

/// Failed OTLP export attempts (one per batch, not per record).
static OTLP_EXPORT_FAILURES: LazyLock<IntCounter> = LazyLock::new(|| {
    IntCounter::new(
        "aviso_otlp_export_failures_total",
        "Failed OTLP log batch export attempts. Each increment is one \
         batch (up to 512 records) that the collector did not accept; \
         those records are dropped from the push path while stdout stays \
         complete. A sustained non-zero rate means the collector endpoint \
         is unreachable or rejecting writes.",
    )
    .expect("metric definition must be valid")
});

/// Register the OTLP export health counters into `registry` so they are
/// served by the application's `/metrics` endpoint.
pub(crate) fn register_otlp_metrics(registry: &Registry) {
    registry
        .register(Box::new(OTLP_SUPPRESSED_LOG_RECORDS.clone()))
        .expect("metric must register");
    registry
        .register(Box::new(OTLP_EXPORT_FAILURES.clone()))
        .expect("metric must register");
}

/// Failures while constructing the OTLP export pipeline at startup.
///
/// These abort startup rather than degrade silently: `logging.otlp.enabled`
/// is explicit operator intent, and a server that quietly drops its log
/// export misleads operators into thinking collection works.
#[derive(Debug, thiserror::Error)]
pub enum OtlpInitError {
    #[error("logging.otlp.enabled=true requires logging.otlp.endpoint")]
    MissingEndpoint,
    #[error("failed to build OTLP log exporter for endpoint {endpoint:?}: {source}")]
    ExporterBuild {
        endpoint: String,
        #[source]
        source: opentelemetry_otlp::ExporterBuildError,
    },
}

/// Build the OTLP bridge layer and its logger provider.
///
/// The returned provider handle must be kept alive for the process lifetime
/// and `shutdown()` must be called on exit to flush buffered records.
pub(super) fn build_layer<S>(
    settings: &OtlpSettings,
    service_name: &str,
) -> Result<(impl Layer<S> + use<S>, SdkLoggerProvider), OtlpInitError>
where
    S: Subscriber + for<'span> LookupSpan<'span>,
{
    let endpoint = settings
        .endpoint
        .as_deref()
        .map(str::trim)
        .filter(|endpoint| !endpoint.is_empty())
        .ok_or(OtlpInitError::MissingEndpoint)?;
    let endpoint = normalize_endpoint(endpoint, settings.protocol);

    let exporter = match settings.protocol {
        OtlpProtocol::Grpc => opentelemetry_otlp::LogExporter::builder()
            .with_tonic()
            .with_endpoint(&endpoint)
            .build(),
        OtlpProtocol::Http => opentelemetry_otlp::LogExporter::builder()
            .with_http()
            .with_endpoint(&endpoint)
            .with_protocol(opentelemetry_otlp::Protocol::HttpBinary)
            .build(),
    }
    .map_err(|source| OtlpInitError::ExporterBuild {
        endpoint: endpoint.clone(),
        source,
    })?;

    let provider = SdkLoggerProvider::builder()
        .with_resource(build_resource(service_name))
        // Processors run in registration order and mutations are visible
        // to the next one: backfill timestamps first, then redact and
        // hand off to the batch exporter.
        .with_log_processor(EventTimestampProcessor)
        .with_log_processor(RedactingLogProcessor::new(
            BatchLogProcessor::builder(MeteredLogExporter::new(exporter)).build(),
        ))
        .build();

    let bridge = OpenTelemetryTracingBridge::new(&provider);
    let layer = bridge.with_filter(export_noise_filter());
    Ok((layer, provider))
}

/// Normalize a configured OTLP endpoint for the chosen transport.
///
/// - A missing scheme defaults to `http://` (in-cluster collectors are
///   typically plaintext; TLS endpoints must spell out `https://`).
/// - For [`OtlpProtocol::Http`] the OTLP logs path `/v1/logs` is appended
///   unless already present, because the exporter uses a programmatically
///   supplied endpoint verbatim; the auto-append only applies to the
///   `OTEL_EXPORTER_OTLP_ENDPOINT` environment variable.
///
/// Valid example: `otel-collector:4317` becomes `http://otel-collector:4317`
/// for gRPC. Invalid example: an endpoint with embedded userinfo such as
/// `http://user:pass@collector:4317` is passed through and will fail URI
/// validation in the exporter builder rather than being silently rewritten.
fn normalize_endpoint(raw: &str, protocol: OtlpProtocol) -> String {
    let with_scheme = if raw.contains("://") {
        raw.to_string()
    } else {
        format!("http://{raw}")
    };
    match protocol {
        OtlpProtocol::Grpc => with_scheme,
        OtlpProtocol::Http => {
            let base = with_scheme.trim_end_matches('/');
            if base.ends_with("/v1/logs") {
                base.to_string()
            } else {
                format!("{base}/v1/logs")
            }
        }
    }
}

/// Resource attributes for exported log records.
///
/// Mirrors the identity block of the stdout formatter (`resource` in the
/// JSON records): service name/version plus pod identity from the same
/// environment variables, so both sinks describe the process identically.
///
/// [`Resource::builder`] starts from the SDK's default detectors, which
/// include the environment detector: attributes supplied via the standard
/// `OTEL_RESOURCE_ATTRIBUTES` variable (for example collector routing
/// keys) are merged automatically, with the explicit attributes below
/// taking precedence on key collision.
fn build_resource(service_name: &str) -> Resource {
    let mut attributes = vec![KeyValue::new(
        "service.version",
        super::SERVICE_VERSION.to_string(),
    )];
    if let Ok(namespace) = std::env::var("K8S_NAMESPACE_NAME") {
        attributes.push(KeyValue::new("k8s.namespace.name", namespace));
    }
    if let Some(pod_name) = std::env::var("K8S_POD_NAME")
        .ok()
        .or_else(|| std::env::var("HOSTNAME").ok())
    {
        attributes.push(KeyValue::new("k8s.pod.name", pod_name));
    }
    Resource::builder()
        .with_service_name(service_name.to_string())
        .with_attributes(attributes)
        .build()
}

/// Targets muted on the OTLP bridge layer to prevent telemetry-induced
/// telemetry: the exporter's own transport stack emits `tracing` events
/// during export, and re-capturing those through the bridge would amplify
/// every export into more log records. The SDK's internal diagnostics use
/// the crate *package* name as target (hyphenated), while module-path
/// targets use underscores, so both spellings are listed where they differ.
///
/// These targets still reach the stdout formatter unfiltered; export
/// failures therefore stay visible in `kubectl logs`.
const EXPORT_NOISE_TARGETS: &[&str] = &[
    "opentelemetry",
    "opentelemetry_sdk",
    "opentelemetry-otlp",
    "opentelemetry_otlp",
    "opentelemetry-appender-tracing",
    "opentelemetry_appender_tracing",
    "tonic",
    "h2",
    "hyper",
    "hyper_util",
    "tower",
    "reqwest",
];

/// Per-layer filter for the bridge: admit everything the global filter
/// admits except the export transport targets.
///
/// The base directive is `trace` so this filter never narrows below the
/// operator's global choice; it only subtracts [`EXPORT_NOISE_TARGETS`].
fn export_noise_filter() -> EnvFilter {
    let mut filter = EnvFilter::new("trace");
    for target in EXPORT_NOISE_TARGETS {
        let directive_str = format!("{target}=off");
        match directive_str.parse() {
            Ok(directive) => filter = filter.add_directive(directive),
            Err(error) => {
                // A bad hardcoded directive is a developer error; surface it
                // on stderr instead of silently weakening loop protection.
                eprintln!(
                    "warning: failed to parse OTLP noise directive {directive_str:?} \
                     ({error}); skipping"
                );
            }
        }
    }
    filter
}

/// Backfill the event timestamp on bridged log records.
///
/// `tracing` events carry no timestamp, so the tracing bridge sets only
/// `observed_timestamp` and leaves `timestamp` unset. OTLP encodes an
/// unset timestamp as 0, which log backends render as 1970-01-01 and
/// which breaks every time-based query over the exported records. Copy
/// the observed timestamp into the event timestamp; for an in-process
/// bridge the two are sub-millisecond apart. Records that already carry
/// an event timestamp are left untouched.
#[derive(Debug)]
struct EventTimestampProcessor;

impl LogProcessor for EventTimestampProcessor {
    fn emit(&self, record: &mut SdkLogRecord, _instrumentation: &InstrumentationScope) {
        if record.timestamp().is_none() {
            let observed = record
                .observed_timestamp()
                .unwrap_or_else(std::time::SystemTime::now);
            record.set_timestamp(observed);
        }
    }

    fn force_flush(&self) -> OTelSdkResult {
        Ok(())
    }

    fn shutdown_with_timeout(&self, _timeout: Duration) -> OTelSdkResult {
        Ok(())
    }
}

/// Log processor that enforces the crate's redaction rules on the push
/// path before records reach the wrapped exporter processor.
///
/// The SDK's public API does not allow rewriting individual attributes on
/// an [`SdkLogRecord`], so attribute redaction cannot mirror the stdout
/// formatter's field-level `[REDACTED]` replacement. Instead this
/// processor is deliberately more conservative: a record carrying a
/// sensitive attribute key (see [`super::is_sensitive_key`]) or a string
/// attribute containing URL userinfo credentials is withheld from export
/// entirely. The stdout pipeline still emits the same record with
/// field-level redaction, so the information is not lost to operators.
///
/// Message bodies can be rewritten (`set_body` is public), so they get the
/// same pattern redaction as stdout bodies.
struct RedactingLogProcessor<P> {
    inner: P,
}

impl<P> RedactingLogProcessor<P> {
    fn new(inner: P) -> Self {
        Self { inner }
    }
}

impl<P: LogProcessor> LogProcessor for RedactingLogProcessor<P> {
    fn emit(&self, record: &mut SdkLogRecord, instrumentation: &InstrumentationScope) {
        if record_carries_sensitive_attributes(record) {
            OTLP_SUPPRESSED_LOG_RECORDS.inc();
            return;
        }
        redact_body(record);
        self.inner.emit(record, instrumentation);
    }

    fn force_flush(&self) -> OTelSdkResult {
        self.inner.force_flush()
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.inner.shutdown_with_timeout(timeout)
    }

    fn event_enabled(&self, level: Severity, target: &str, name: Option<&str>) -> bool {
        self.inner.event_enabled(level, target, name)
    }

    fn set_resource(&mut self, resource: &Resource) {
        // Forwarding is load-bearing: the batch processor snapshots the
        // resource here; dropping the call would strip service identity
        // from every exported record.
        self.inner.set_resource(resource);
    }
}

impl<P> std::fmt::Debug for RedactingLogProcessor<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedactingLogProcessor")
            .finish_non_exhaustive()
    }
}

/// Exporter wrapper that counts failed export attempts.
///
/// The batch processor logs export failures through the SDK's internal
/// diagnostics (visible on stdout), but a grep is not an alerting
/// surface; the counter makes "log export is broken" a Prometheus alert.
#[derive(Debug)]
struct MeteredLogExporter<E> {
    inner: E,
}

impl<E> MeteredLogExporter<E> {
    fn new(inner: E) -> Self {
        Self { inner }
    }
}

impl<E: LogExporter> LogExporter for MeteredLogExporter<E> {
    async fn export(&self, batch: LogBatch<'_>) -> OTelSdkResult {
        let result = self.inner.export(batch).await;
        if result.is_err() {
            OTLP_EXPORT_FAILURES.inc();
        }
        result
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.inner.shutdown_with_timeout(timeout)
    }

    fn event_enabled(&self, level: Severity, target: &str, name: Option<&str>) -> bool {
        self.inner.event_enabled(level, target, name)
    }

    fn set_resource(&mut self, resource: &Resource) {
        // Forwarding is load-bearing: the OTLP exporter snapshots the
        // resource here for every outgoing payload.
        self.inner.set_resource(resource);
    }
}

fn record_carries_sensitive_attributes(record: &SdkLogRecord) -> bool {
    record.attributes_iter().any(|(key, value)| {
        if super::is_sensitive_key(key.as_str()) {
            return true;
        }
        match value {
            AnyValue::String(s) => super::redact_url_userinfo(s.as_str()).is_some(),
            _ => false,
        }
    })
}

fn redact_body(record: &mut SdkLogRecord) {
    let redacted = match record.body() {
        Some(AnyValue::String(s)) => {
            let redacted = super::redact_message(s.as_str());
            (redacted != s.as_str()).then_some(redacted)
        }
        _ => None,
    };
    if let Some(redacted) = redacted {
        record.set_body(AnyValue::String(redacted.into()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::logs::{LogRecord, Logger, LoggerProvider};
    use std::sync::{Arc, Mutex};

    #[test]
    fn normalize_endpoint_adds_scheme_when_missing() {
        assert_eq!(
            normalize_endpoint("collector:4317", OtlpProtocol::Grpc),
            "http://collector:4317"
        );
        assert_eq!(
            normalize_endpoint("http://collector:4317", OtlpProtocol::Grpc),
            "http://collector:4317"
        );
        assert_eq!(
            normalize_endpoint("https://collector:4317", OtlpProtocol::Grpc),
            "https://collector:4317"
        );
    }

    #[test]
    fn normalize_endpoint_appends_logs_path_for_http() {
        assert_eq!(
            normalize_endpoint("http://collector:4318", OtlpProtocol::Http),
            "http://collector:4318/v1/logs"
        );
        assert_eq!(
            normalize_endpoint("http://collector:4318/", OtlpProtocol::Http),
            "http://collector:4318/v1/logs"
        );
        assert_eq!(
            normalize_endpoint("http://collector:4318/v1/logs", OtlpProtocol::Http),
            "http://collector:4318/v1/logs"
        );
        assert_eq!(
            normalize_endpoint("collector:4318", OtlpProtocol::Http),
            "http://collector:4318/v1/logs"
        );
    }

    #[test]
    fn export_noise_targets_assemble_to_valid_directives() {
        // Mirrors the default-mute-directive test in the parent module: a
        // typo here would silently disappear at runtime via the
        // skip-and-eprintln branch, so pin parseability at test time.
        for target in EXPORT_NOISE_TARGETS {
            let directive_str = format!("{target}=off");
            directive_str
                .parse::<tracing_subscriber::filter::Directive>()
                .unwrap_or_else(|error| {
                    panic!("noise directive {directive_str:?} must parse: {error}")
                });
        }
    }

    #[test]
    fn missing_endpoint_fails_layer_construction() {
        let settings = OtlpSettings {
            enabled: true,
            endpoint: None,
            protocol: OtlpProtocol::Grpc,
        };
        let result = build_layer::<tracing_subscriber::Registry>(&settings, "test");
        assert!(matches!(result, Err(OtlpInitError::MissingEndpoint)));

        let settings = OtlpSettings {
            enabled: true,
            endpoint: Some("   ".to_string()),
            protocol: OtlpProtocol::Grpc,
        };
        let result = build_layer::<tracing_subscriber::Registry>(&settings, "test");
        assert!(matches!(result, Err(OtlpInitError::MissingEndpoint)));
    }

    /// Inner processor that records every forwarded record and resource
    /// call, so tests can observe exactly what the redacting wrapper lets
    /// through.
    #[derive(Debug, Clone, Default)]
    struct CapturingProcessor {
        records: Arc<Mutex<Vec<SdkLogRecord>>>,
        resources: Arc<Mutex<Vec<Resource>>>,
    }

    impl LogProcessor for CapturingProcessor {
        fn emit(&self, record: &mut SdkLogRecord, _instrumentation: &InstrumentationScope) {
            self.records
                .lock()
                .expect("test lock poisoned")
                .push(record.clone());
        }

        fn force_flush(&self) -> OTelSdkResult {
            Ok(())
        }

        fn shutdown_with_timeout(&self, _timeout: Duration) -> OTelSdkResult {
            Ok(())
        }

        fn set_resource(&mut self, resource: &Resource) {
            self.resources
                .lock()
                .expect("test lock poisoned")
                .push(resource.clone());
        }
    }

    /// Build a record through the public logger API; `SdkLogRecord` has no
    /// public constructor.
    fn new_record() -> SdkLogRecord {
        SdkLoggerProvider::builder()
            .build()
            .logger("test")
            .create_log_record()
    }

    fn scope() -> InstrumentationScope {
        InstrumentationScope::builder("test").build()
    }

    fn emitted_through_wrapper(record: &mut SdkLogRecord) -> Vec<SdkLogRecord> {
        let capturing = CapturingProcessor::default();
        let records = capturing.records.clone();
        let wrapper = RedactingLogProcessor::new(capturing);
        wrapper.emit(record, &scope());
        let captured = records.lock().expect("test lock poisoned");
        captured.clone()
    }

    #[test]
    fn benign_record_passes_through_unchanged() {
        let mut record = new_record();
        record.set_body(AnyValue::String("request handled".into()));
        record.add_attribute("event.name", "api.request.succeeded");

        let forwarded = emitted_through_wrapper(&mut record);
        assert_eq!(forwarded.len(), 1);
        assert_eq!(
            forwarded[0].body(),
            Some(&AnyValue::String("request handled".into()))
        );
    }

    #[test]
    fn sensitive_attribute_key_suppresses_record_and_counts_it() {
        let suppressed_before = OTLP_SUPPRESSED_LOG_RECORDS.get();
        let mut record = new_record();
        record.set_body(AnyValue::String("auth check".into()));
        record.add_attribute("api_key", "super-secret");

        let forwarded = emitted_through_wrapper(&mut record);
        assert!(
            forwarded.is_empty(),
            "records with sensitive attribute keys must not be exported"
        );
        // Strict-greater comparison: other tests exercising suppression run
        // in parallel against the same process-wide counter.
        assert!(
            OTLP_SUPPRESSED_LOG_RECORDS.get() > suppressed_before,
            "suppression must increment the counter"
        );
    }

    #[test]
    fn url_userinfo_attribute_value_suppresses_record() {
        let mut record = new_record();
        record.set_body(AnyValue::String("backend connected".into()));
        record.add_attribute("nats_url", "nats://user:pass@nats:4222");

        let forwarded = emitted_through_wrapper(&mut record);
        assert!(
            forwarded.is_empty(),
            "records with credentialed URLs in attributes must not be exported"
        );
    }

    #[test]
    fn credential_free_url_attribute_is_not_suppressed() {
        let mut record = new_record();
        record.set_body(AnyValue::String("backend connected".into()));
        record.add_attribute("nats_url", "nats://nats:4222");

        let forwarded = emitted_through_wrapper(&mut record);
        assert_eq!(forwarded.len(), 1);
    }

    #[test]
    fn body_is_pattern_redacted_before_export() {
        let mut record = new_record();
        record.set_body(AnyValue::String("login with password=hunter2 done".into()));

        let forwarded = emitted_through_wrapper(&mut record);
        assert_eq!(forwarded.len(), 1);
        assert_eq!(
            forwarded[0].body(),
            Some(&AnyValue::String(
                "login with password=[REDACTED] done".into()
            ))
        );
    }

    #[test]
    fn set_resource_is_forwarded_to_inner_processor() {
        // Losing this delegation would silently strip service identity
        // from every exported record; pin it.
        let capturing = CapturingProcessor::default();
        let resources = capturing.resources.clone();
        let mut wrapper = RedactingLogProcessor::new(capturing);
        let resource = Resource::builder().with_service_name("svc").build();
        wrapper.set_resource(&resource);
        assert_eq!(resources.lock().expect("test lock poisoned").len(), 1);
    }

    /// Exporter stub with a scripted outcome, for exercising the metered
    /// wrapper without a network.
    #[derive(Debug)]
    struct ScriptedExporter {
        fail: bool,
    }

    impl LogExporter for ScriptedExporter {
        async fn export(&self, _batch: LogBatch<'_>) -> OTelSdkResult {
            if self.fail {
                Err(opentelemetry_sdk::error::OTelSdkError::InternalFailure(
                    "scripted failure".to_string(),
                ))
            } else {
                Ok(())
            }
        }
    }

    fn export_once(exporter: &MeteredLogExporter<ScriptedExporter>) -> OTelSdkResult {
        let record = new_record();
        let scope = scope();
        let data = [(&record, &scope)];
        futures::executor::block_on(exporter.export(LogBatch::new(&data)))
    }

    #[test]
    fn metered_exporter_counts_failed_exports_only() {
        // Exact deltas are safe here: this test is the only user of
        // OTLP_EXPORT_FAILURES in the test binary (suppression tests touch
        // the other counter). If that changes, these assertions will fail
        // loudly rather than silently weakening.
        let failures_before = OTLP_EXPORT_FAILURES.get();

        let failing = MeteredLogExporter::new(ScriptedExporter { fail: true });
        assert!(export_once(&failing).is_err());
        assert_eq!(
            OTLP_EXPORT_FAILURES.get(),
            failures_before + 1,
            "failed export must increment the counter"
        );

        let succeeding = MeteredLogExporter::new(ScriptedExporter { fail: false });
        assert!(export_once(&succeeding).is_ok());
        assert_eq!(
            OTLP_EXPORT_FAILURES.get(),
            failures_before + 1,
            "successful export must not increment the counter"
        );
    }

    #[test]
    fn event_timestamp_backfilled_from_observed_timestamp() {
        // The bridge leaves `timestamp` unset; OTLP encodes that as 0 and
        // backends render 1970-01-01. The processor must copy the
        // observed timestamp so time-based queries work.
        let observed = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_785_176_180);
        let mut record = new_record();
        record.set_observed_timestamp(observed);
        assert!(record.timestamp().is_none(), "precondition: no timestamp");

        EventTimestampProcessor.emit(&mut record, &scope());
        assert_eq!(record.timestamp(), Some(observed));
    }

    #[test]
    fn existing_event_timestamp_is_left_untouched() {
        let event_time = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        let observed = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(2_000);
        let mut record = new_record();
        record.set_timestamp(event_time);
        record.set_observed_timestamp(observed);

        EventTimestampProcessor.emit(&mut record, &scope());
        assert_eq!(record.timestamp(), Some(event_time));
    }

    #[test]
    fn event_timestamp_falls_back_to_now_without_observed_timestamp() {
        // The SDK logger sets observed_timestamp before processors run,
        // but the processor must not rely on that ordering.
        let before = std::time::SystemTime::now();
        let mut record = new_record();
        assert!(record.observed_timestamp().is_none());

        EventTimestampProcessor.emit(&mut record, &scope());
        let after = std::time::SystemTime::now();
        let stamped = record.timestamp().expect("timestamp must be set");
        assert!(stamped >= before && stamped <= after);
    }

    #[test]
    fn register_otlp_metrics_exposes_both_counters() {
        let registry = Registry::new();
        register_otlp_metrics(&registry);

        use prometheus::Encoder;
        let mut buf = Vec::new();
        prometheus::TextEncoder::new()
            .encode(&registry.gather(), &mut buf)
            .expect("encode ok");
        let output = String::from_utf8(buf).expect("valid utf8");

        assert!(output.contains("aviso_otlp_suppressed_log_records_total"));
        assert!(output.contains("aviso_otlp_export_failures_total"));
    }
}
