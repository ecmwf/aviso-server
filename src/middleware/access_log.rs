// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! `RootSpanBuilder` that demotes infrastructure-path request spans to debug.
//!
//! `tracing-actix-web`'s default builder creates an info-level root span for
//! every HTTP request. With the standard subscriber that emits span events on
//! `FmtSpan::CLOSE` (one log line per request close), this floods logs from
//! high-frequency infrastructure paths: Kubernetes liveness probes on
//! `/health`, Prometheus scrapes on `/metrics`, static asset fetches, Swagger
//! UI loads, OpenAPI doc fetches.
//!
//! What this builder changes — and what it does NOT change:
//!
//! * **Span emission events** (close events from `FmtSpan::CLOSE` /
//!   `FmtSpan::ACTIVE` / etc.) for the request span are filtered at the
//!   span's own level. By emitting the request span at debug for
//!   infrastructure paths, those span-close lines are silenced at the
//!   default info filter while business-route span-close lines stay
//!   visible. Today aviso uses `FmtSpan::NONE` so this saves zero lines;
//!   the value is realised the moment `FmtSpan` is reconfigured (an SRE
//!   tweak that could ship in any future PR for richer trace correlation),
//!   without re-introducing the access-log flood at the same time.
//!
//! * **Application events recorded by handlers inside the request lifetime**
//!   are *unaffected*. Each event is filtered by its own call-site level
//!   (the `tracing::info!` / `warn!` / `error!` macro the handler chose),
//!   independent of the parent span's level. A handler that emits a
//!   `tracing::error!` from inside a `/health` request still logs that
//!   error.
//!
//! * **OpenTelemetry parent propagation** for the request is delegated to
//!   `DefaultRootSpanBuilder::on_request_end` and inherits the span's level
//!   for trace export. For deployments with OTel disabled (the aviso
//!   default today), this is a no-op.
//!
//! In other words: Phase 1 is a span-level pin, not an event-level pin. It
//! exists to prevent the request span itself from generating noise on
//! infrastructure paths if the subscriber's `FmtSpan` config ever changes,
//! without affecting handler-emitted events.

use actix_web::Error;
use actix_web::body::MessageBody;
use actix_web::dev::{ServiceRequest, ServiceResponse};
use tracing::Span;
use tracing_actix_web::{DefaultRootSpanBuilder, Level, RootSpanBuilder, root_span};

pub struct AvisoRootSpanBuilder;

impl RootSpanBuilder for AvisoRootSpanBuilder {
    fn on_request_start(request: &ServiceRequest) -> Span {
        if is_infrastructure_path(request.uri().path()) {
            root_span!(level = Level::DEBUG, request)
        } else {
            root_span!(request)
        }
    }

    fn on_request_end<B: MessageBody>(span: Span, outcome: &Result<ServiceResponse<B>, Error>) {
        DefaultRootSpanBuilder::on_request_end(span, outcome);
    }
}

/// Paths whose request span is recorded at debug instead of info.
///
/// Curation rules: only routes whose successful traffic is high-frequency
/// AND carries no operator-relevant signal beyond what existing Prometheus
/// metrics or Kubernetes events already surface. Adding routes here is a
/// behaviour change for log-based alerting; do not extend without auditing
/// downstream consumers first.
fn is_infrastructure_path(path: &str) -> bool {
    path == "/health"
        || path == "/metrics"
        || path.starts_with("/static/")
        || path.starts_with("/swagger-ui/")
        || path.starts_with("/api-docs/")
}

#[cfg(test)]
mod tests {
    use super::is_infrastructure_path;

    #[test]
    fn infrastructure_paths_are_quiet() {
        assert!(is_infrastructure_path("/health"));
        assert!(is_infrastructure_path("/metrics"));
        assert!(is_infrastructure_path("/static/logo.png"));
        assert!(is_infrastructure_path("/swagger-ui/"));
        assert!(is_infrastructure_path("/swagger-ui/index.html"));
        assert!(is_infrastructure_path("/api-docs/openapi.json"));
    }

    #[test]
    fn business_paths_remain_loud() {
        assert!(!is_infrastructure_path("/"));
        assert!(!is_infrastructure_path("/api/v1/notification"));
        assert!(!is_infrastructure_path("/api/v1/watch"));
        assert!(!is_infrastructure_path("/api/v1/replay"));
        assert!(!is_infrastructure_path("/api/v1/schema"));
        assert!(!is_infrastructure_path("/api/v1/admin/wipe/all"));
    }

    #[test]
    fn near_misses_remain_loud() {
        // Substring matches must not accidentally match infrastructure paths.
        // /healthcheck looks like /health but isn't; if we ever add such a
        // route, it stays at info until explicitly added to the quiet list.
        assert!(!is_infrastructure_path("/healthcheck"));
        assert!(!is_infrastructure_path("/healthz"));
        assert!(!is_infrastructure_path("/metricsfoo"));
        // /health/ trailing-slash: the builder runs BEFORE Actix routing, so
        // this path does reach is_infrastructure_path and we explicitly
        // choose not to match it. The exact-equality match on "/health" is
        // intentional: a non-empty trailing slash usually means the operator
        // typo'd the URL, and Actix returns 404 (no NormalizePath wrapper
        // is registered), so the resulting log line is genuinely
        // diagnostic for "someone is hitting the wrong path" and should
        // stay at the default info level rather than be silently muted.
        assert!(!is_infrastructure_path("/health/"));
    }
}
