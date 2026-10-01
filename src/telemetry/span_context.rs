// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Request context fields as recorded on spans.
//!
//! [`SpanContextLayer`] keeps the values of `super::HYDRATABLE_SPAN_FIELDS`
//! that each span records, so the stdout formatter can attach them to the
//! events inside the span without parsing the span's formatted text. A value
//! recorded again replaces the earlier one, as `Span::record` means.

use serde_json::{Map, Value, json};
use tracing::Subscriber;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

/// The request context values one span has recorded, keyed by field name.
#[derive(Debug, Default)]
pub(super) struct SpanContext(Map<String, Value>);

impl SpanContext {
    pub(super) fn get(&self, field: &str) -> Option<&Value> {
        self.0.get(field)
    }
}

/// Stores each span's request context values in the span's extensions.
pub(super) struct SpanContextLayer;

impl<S> Layer<S> for SpanContextLayer
where
    S: Subscriber + for<'span> LookupSpan<'span>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let mut context = SpanContext::default();
        attrs.record(&mut ContextVisitor(&mut context.0));
        if context.0.is_empty() {
            return;
        }
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(context);
        }
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else {
            return;
        };
        let mut extensions = span.extensions_mut();
        if let Some(context) = extensions.get_mut::<SpanContext>() {
            values.record(&mut ContextVisitor(&mut context.0));
            return;
        }
        let mut context = SpanContext::default();
        values.record(&mut ContextVisitor(&mut context.0));
        if !context.0.is_empty() {
            extensions.insert(context);
        }
    }
}

/// Records the request context fields only, with the OTLP bridge's
/// conversions so both sinks show the same value: strings as they are,
/// numbers and booleans as such, errors by their `Display` text, and other
/// values through `Debug`, which for a `%value` field prints its `Display`
/// form. The field name is checked first, so other span fields are never
/// formatted.
struct ContextVisitor<'a>(&'a mut Map<String, Value>);

impl ContextVisitor<'_> {
    fn wanted(field: &Field) -> bool {
        super::HYDRATABLE_SPAN_FIELDS.contains(&field.name())
    }

    fn keep(&mut self, field: &Field, value: impl FnOnce() -> Value) {
        if Self::wanted(field) {
            self.0.insert(field.name().to_string(), value());
        }
    }
}

impl Visit for ContextVisitor<'_> {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.keep(field, || json!(value));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.keep(field, || json!(value));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.keep(field, || json!(value));
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.keep(field, || json!(value));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.keep(field, || json!(value));
    }

    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.keep(field, || json!(value.to_string()));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.keep(field, || json!(format!("{value:?}")));
    }
}
