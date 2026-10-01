// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Keeps one value per attribute key on exported log records.
//!
//! The OTLP bridge copies the request context fields from every span in
//! scope, outermost first, and then adds the event's own fields. A field set
//! on two nested spans, or on a span and on the event, therefore arrives
//! more than once. The stdout formatter takes the event's value, else the
//! innermost span's; keeping the last occurrence of each key gives exported
//! records the same value.

use std::collections::HashMap;
use std::time::Duration;

use opentelemetry::InstrumentationScope;
use opentelemetry::logs::{LogRecord, Logger, LoggerProvider};
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::logs::{LogProcessor, SdkLogRecord, SdkLogger, SdkLoggerProvider};

/// Log processor that removes repeated attribute keys, keeping the last value
/// of each.
///
/// Records without a repeated key pass through untouched. The SDK cannot
/// remove an attribute from a record, so a record with a repeated key is
/// rebuilt: every other field is copied, and the attributes are kept in the
/// order of their last occurrence.
pub(super) struct UniqueAttributesProcessor {
    /// Source of empty records; `SdkLogRecord` has no public constructor.
    /// The provider behind it has no processors, so nothing is emitted.
    blank: SdkLogger,
}

impl UniqueAttributesProcessor {
    pub(super) fn new() -> Self {
        Self {
            blank: SdkLoggerProvider::builder()
                .build()
                .logger("aviso-unique-attributes"),
        }
    }

    fn rebuilt(&self, record: &SdkLogRecord) -> SdkLogRecord {
        let attributes: Vec<_> = record.attributes_iter().collect();
        let mut last = HashMap::with_capacity(attributes.len());
        for (index, (key, _)) in attributes.iter().enumerate() {
            last.insert(key.clone(), index);
        }

        let mut copy = self.blank.create_log_record();
        if let Some(name) = record.event_name() {
            copy.set_event_name(name);
        }
        if let Some(target) = record.target() {
            copy.set_target(target.clone());
        }
        if let Some(timestamp) = record.timestamp() {
            copy.set_timestamp(timestamp);
        }
        if let Some(observed) = record.observed_timestamp() {
            copy.set_observed_timestamp(observed);
        }
        if let Some(context) = record.trace_context() {
            copy.set_trace_context(context.trace_id, context.span_id, context.trace_flags);
        }
        if let Some(text) = record.severity_text() {
            copy.set_severity_text(text);
        }
        if let Some(number) = record.severity_number() {
            copy.set_severity_number(number);
        }
        if let Some(body) = record.body() {
            copy.set_body(body.clone());
        }
        for (index, (key, value)) in attributes.into_iter().enumerate() {
            if last.get(key) == Some(&index) {
                copy.add_attribute(key.clone(), value.clone());
            }
        }
        copy
    }
}

fn has_repeated_key(record: &SdkLogRecord) -> bool {
    let mut seen = std::collections::HashSet::new();
    record.attributes_iter().any(|(key, _)| !seen.insert(key))
}

impl LogProcessor for UniqueAttributesProcessor {
    fn emit(&self, record: &mut SdkLogRecord, _instrumentation: &InstrumentationScope) {
        if has_repeated_key(record) {
            *record = self.rebuilt(record);
        }
    }

    fn force_flush(&self) -> OTelSdkResult {
        Ok(())
    }

    fn shutdown_with_timeout(&self, _timeout: Duration) -> OTelSdkResult {
        Ok(())
    }
}

impl std::fmt::Debug for UniqueAttributesProcessor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UniqueAttributesProcessor")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::logs::{AnyValue, Severity};
    use opentelemetry::trace::{SpanId, TraceFlags, TraceId};
    use std::time::SystemTime;

    fn record() -> SdkLogRecord {
        SdkLoggerProvider::builder()
            .build()
            .logger("test")
            .create_log_record()
    }

    fn emitted(record: &mut SdkLogRecord) {
        UniqueAttributesProcessor::new()
            .emit(record, &InstrumentationScope::builder("test").build());
    }

    fn attributes(record: &SdkLogRecord) -> Vec<(String, AnyValue)> {
        record
            .attributes_iter()
            .map(|(key, value)| (key.to_string(), value.clone()))
            .collect()
    }

    #[test]
    fn record_without_repeated_keys_is_unchanged() {
        let mut original = record();
        original.add_attribute("request_id", "r1");
        original.add_attribute("username", "alice");
        let mut forwarded = original.clone();
        emitted(&mut forwarded);
        assert_eq!(attributes(&forwarded), attributes(&original));
    }

    #[test]
    fn repeated_key_keeps_its_last_value_and_every_other_field() {
        let timestamp = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
        let observed = SystemTime::UNIX_EPOCH + Duration::from_secs(11);
        let trace_id = TraceId::from_hex("0af7651916cd43dd8448eb211c80319c").unwrap();
        let span_id = SpanId::from_hex("b7ad6b7169203331").unwrap();
        let mut rec = record();
        rec.set_event_name("event src/routes/replay.rs:1");
        rec.set_target("aviso_server::routes::replay");
        rec.set_timestamp(timestamp);
        rec.set_observed_timestamp(observed);
        rec.set_trace_context(trace_id, span_id, Some(TraceFlags::SAMPLED));
        rec.set_severity_text("INFO");
        rec.set_severity_number(Severity::Info);
        rec.set_body(AnyValue::String("replay started".into()));
        // Outer span, inner span, then the event, as the bridge adds them.
        rec.add_attribute("request_id", "r1");
        rec.add_attribute("event_type", "outer");
        rec.add_attribute("username", "alice");
        rec.add_attribute("event_type", "inner");
        rec.add_attribute("request_id", "r1");
        rec.add_attribute("event_type", "event");

        emitted(&mut rec);

        assert_eq!(
            attributes(&rec),
            vec![
                ("username".to_string(), AnyValue::from("alice")),
                ("request_id".to_string(), AnyValue::from("r1")),
                ("event_type".to_string(), AnyValue::from("event")),
            ]
        );
        assert_eq!(rec.event_name(), Some("event src/routes/replay.rs:1"));
        assert_eq!(
            rec.target().map(AsRef::as_ref),
            Some("aviso_server::routes::replay")
        );
        assert_eq!(rec.timestamp(), Some(timestamp));
        assert_eq!(rec.observed_timestamp(), Some(observed));
        let context = rec.trace_context().expect("trace context kept");
        assert_eq!(context.trace_id, trace_id);
        assert_eq!(context.span_id, span_id);
        assert_eq!(context.trace_flags, Some(TraceFlags::SAMPLED));
        assert_eq!(rec.severity_text(), Some("INFO"));
        assert_eq!(rec.severity_number(), Some(Severity::Info));
        assert_eq!(rec.body(), Some(&AnyValue::String("replay started".into())));
    }
}
