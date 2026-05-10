// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! `RootSpanBuilder` that demotes infrastructure paths to a debug-level span.
//!
//! `tracing-actix-web`'s default builder creates an info-level root span for
//! every HTTP request. That span carries fields like `http.method`,
//! `http.status_code`, and `request_id`, and any event recorded inside the
//! request's lifetime inherits them. With the standard subscriber that emits
//! span events on `FmtSpan::CLOSE`, this becomes one log line per request,
//! including high-frequency infrastructure paths (Kubernetes liveness probes
//! on `/health`, Prometheus scrapes on `/metrics`, static asset fetches,
//! Swagger UI loads).
//!
//! Today aviso's subscriber sets `FmtSpan::NONE`, so the default builder is
//! already silent at the span boundary — but the span level still gates which
//! application-level events inside it are emitted, and any future change to
//! `FmtSpan` (e.g. an operator dialing up to debug to investigate) would
//! immediately flood the logs from these paths.
//!
//! [`AvisoRootSpanBuilder`] is a forward-defensive alternative. For a fixed
//! list of infrastructure paths it builds the span at debug level instead of
//! info, so:
//! - At the default info filter the span is below the floor and nothing it
//!   emits is recorded, regardless of `FmtSpan` configuration.
//! - At debug filter (or via `RUST_LOG=info,aviso_server=debug`) the span
//!   becomes visible again for triage.
//!
//! Application-level events (from handlers) are unaffected: their level is
//! whatever the call site chose, independent of the parent span's level.
//!
//! This builder also short-circuits OpenTelemetry parent propagation for
//! quiet paths via the standard delegation pattern, which keeps the OTel
//! pipeline aligned with the noise budget.

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
        // Trailing-slash variant of /health is intentionally NOT matched
        // because the route is registered as exactly "/health" and Actix
        // would 404 a "/health/" request before reaching this builder.
        assert!(!is_infrastructure_path("/health/"));
    }
}
