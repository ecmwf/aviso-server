// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use crate::middleware::access_log::AvisoRootSpanBuilder;
use crate::middleware::request_id::RequestIdHeader;
use crate::telemetry::SERVICE_VERSION;
use actix_web::{App, HttpResponse, HttpServer, dev::Server, web};
use prometheus::{
    Encoder, Histogram, HistogramVec, IntCounter, IntCounterVec, IntGaugeVec, Registry,
    TextEncoder, histogram_opts, opts, register_histogram_vec_with_registry,
    register_int_counter_vec_with_registry, register_int_gauge_vec_with_registry,
};
#[cfg(feature = "ecpds")]
use prometheus::{IntGauge, register_int_counter_with_registry, register_int_gauge_with_registry};
use std::collections::HashMap;
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tracing_actix_web::TracingLogger;

/// Feature-gated ECPDS authorization plugin metrics.
///
/// Recorded by the route layer (`enforce_ecpds_auth`); the subcrate
/// itself stays framework-agnostic. Per-server fetch
/// success/failure/duration is published as structured `tracing`
/// events under `auth.ecpds.fetch.*` so log-based monitoring can pick
/// them up without coupling the subcrate to a metrics backend.
#[cfg(feature = "ecpds")]
#[derive(Clone, Debug)]
pub struct EcpdsMetrics {
    pub cache_hits_total: IntCounter,
    pub cache_misses_total: IntCounter,
    pub cache_size: IntGauge,
    pub access_decisions_total: IntCounterVec,
    pub fetch_total: IntCounterVec,
}

/// Application-level metrics registered in a shared Prometheus registry.
#[derive(Clone, Debug)]
pub struct AppMetrics {
    pub registry: Registry,
    pub build_info: IntGaugeVec,
    pub http_requests_total: IntCounterVec,
    pub http_request_duration_seconds: HistogramVec,
    pub notifications_total: IntCounterVec,
    pub sse_connections_active: IntGaugeVec,
    pub sse_connections_total: IntCounterVec,
    pub sse_unique_users_active: IntGaugeVec,
    pub sse_events_sent_total: IntCounterVec,
    pub sse_stream_errors_total: IntCounterVec,
    pub sse_connection_duration_seconds: HistogramVec,
    pub auth_requests_total: IntCounterVec,
    #[cfg(feature = "ecpds")]
    pub ecpds: EcpdsMetrics,
    unique_users: Arc<Mutex<HashMap<String, HashMap<String, usize>>>>,
}

impl Default for AppMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl AppMetrics {
    pub fn new() -> Self {
        let registry = Registry::new();

        // Constant-1 gauge carrying the crate version as a label. Dashboards
        // join on it to annotate deploys and correlate behaviour changes with
        // rollouts (standard Prometheus `*_build_info` convention).
        let build_info = register_int_gauge_vec_with_registry!(
            opts!(
                "aviso_build_info",
                "Build information; constant 1 with the server version as a label"
            ),
            &["version"],
            registry
        )
        .expect("metric must register");
        build_info.with_label_values(&[SERVICE_VERSION]).set(1);

        let http_requests_total = register_int_counter_vec_with_registry!(
            opts!(
                "aviso_http_requests_total",
                "HTTP requests by matched route pattern, method, and status code. Two reserved endpoint values bound label cardinality: unrouted requests (404 scans) collapse into endpoint=\"unmatched\", and requests whose handling failed with a service-level error (no route information available) record endpoint=\"error\"."
            ),
            &["endpoint", "method", "status_code"],
            registry
        )
        .expect("metric must register");

        let http_request_duration_seconds = register_histogram_vec_with_registry!(
            histogram_opts!(
                "aviso_http_request_duration_seconds",
                "HTTP request duration in seconds by matched route pattern and method, measured until response headers are ready. For SSE endpoints (watch/replay) this is stream setup latency, NOT connection lifetime; see aviso_sse_connection_duration_seconds for that."
            ),
            &["endpoint", "method"],
            registry
        )
        .expect("metric must register");

        let notifications_total = register_int_counter_vec_with_registry!(
            opts!(
                "aviso_notifications_total",
                "Total notification requests by event type and outcome"
            ),
            &["event_type", "status"],
            registry
        )
        .expect("metric must register");
        // Pre-initialise the bounded label values so the series exist at zero
        // from process startup; see the ECPDS pre-init comment below for why
        // missing series break `rate(...) > 0` alert rules. Requests that fail
        // before schema validation are recorded under event_type="unknown" and
        // can only be errors or auth rejections, never successes. Per-stream
        // series are pre-initialised via `preinit_notification_series` once
        // the schema is loaded.
        for status in ["error", "rejected"] {
            let _ = notifications_total.with_label_values(&["unknown", status]);
        }

        let sse_connections_active = register_int_gauge_vec_with_registry!(
            opts!(
                "aviso_sse_connections_active",
                "Currently active SSE connections"
            ),
            &["endpoint", "event_type"],
            registry
        )
        .expect("metric must register");

        let sse_connections_total = register_int_counter_vec_with_registry!(
            opts!(
                "aviso_sse_connections_total",
                "Total SSE connections opened"
            ),
            &["endpoint", "event_type"],
            registry
        )
        .expect("metric must register");

        let sse_unique_users_active = register_int_gauge_vec_with_registry!(
            opts!(
                "aviso_sse_unique_users_active",
                "Distinct users with active SSE connections"
            ),
            &["endpoint"],
            registry
        )
        .expect("metric must register");

        let sse_events_sent_total = register_int_counter_vec_with_registry!(
            opts!(
                "aviso_sse_events_sent_total",
                "Notification events delivered to SSE clients. Counts only notification frames; heartbeats, control events (connection_established, replay_started/completed/limit_reached), and close frames are excluded."
            ),
            &["endpoint", "event_type"],
            registry
        )
        .expect("metric must register");

        let sse_stream_errors_total = register_int_counter_vec_with_registry!(
            opts!(
                "aviso_sse_stream_errors_total",
                "Error events emitted into SSE streams after the response started (typed stream errors and notification rendering failures). These failures are invisible to HTTP status metrics because the stream already returned 200."
            ),
            &["endpoint", "event_type"],
            registry
        )
        .expect("metric must register");

        let sse_connection_duration_seconds = register_histogram_vec_with_registry!(
            histogram_opts!(
                "aviso_sse_connection_duration_seconds",
                "SSE connection lifetime in seconds, observed only when the connection closes; long-lived open connections appear in aviso_sse_connections_active, not here.",
                vec![
                    1.0, 5.0, 15.0, 30.0, 60.0, 300.0, 900.0, 1800.0, 3600.0, 7200.0, 14400.0,
                    28800.0, 43200.0, 86400.0
                ]
            ),
            &["endpoint"],
            registry
        )
        .expect("metric must register");

        let auth_requests_total = register_int_counter_vec_with_registry!(
            opts!(
                "aviso_auth_requests_total",
                "Authentication attempts by mode and outcome"
            ),
            &["mode", "outcome"],
            registry
        )
        .expect("metric must register");
        for mode in ["direct", "trusted_proxy"] {
            for outcome in [
                "success",
                "unauthorized",
                "forbidden",
                "service_unavailable",
            ] {
                let _ = auth_requests_total.with_label_values(&[mode, outcome]);
            }
        }

        #[cfg(feature = "ecpds")]
        let ecpds = {
            let metrics = EcpdsMetrics {
                cache_hits_total: register_int_counter_with_registry!(
                    opts!(
                        "aviso_ecpds_cache_hits_total",
                        "ECPDS destination cache hits"
                    ),
                    registry
                )
                .expect("metric must register"),
                cache_misses_total: register_int_counter_with_registry!(
                    opts!(
                        "aviso_ecpds_cache_misses_total",
                        "ECPDS destination cache misses (request not served from cache; an upstream fetch ran for this caller or a concurrent caller via single-flight)"
                    ),
                    registry
                )
                .expect("metric must register"),
                cache_size: register_int_gauge_with_registry!(
                    opts!(
                        "aviso_ecpds_cache_size",
                        "Number of usernames held in the ECPDS destination cache (sampled from moka after eviction passes; may include not-yet-pruned expired entries until the next pending-tasks run)"
                    ),
                    registry
                )
                .expect("metric must register"),
                access_decisions_total: register_int_counter_vec_with_registry!(
                    opts!(
                        "aviso_ecpds_access_decisions_total",
                        "ECPDS access check outcomes"
                    ),
                    &["outcome"],
                    registry
                )
                .expect("metric must register"),
                fetch_total: register_int_counter_vec_with_registry!(
                    opts!(
                        "aviso_ecpds_fetch_total",
                        "ECPDS upstream fetch outcomes; incremented exactly once per upstream call (the request whose check actually ran the fetch). Coalesced waiters that joined an in-flight fetch are NOT counted, so this counter measures actual upstream call volume rather than per-request fetch attempts."
                    ),
                    &["outcome"],
                    registry
                )
                .expect("metric must register"),
            };
            // Pre-initialise every label value of the labelled counters
            // so the corresponding Prometheus series exist at zero from
            // process startup. Without this, alert rules of the form
            // `rate(metric{outcome="error"}[5m]) > 0` silently fail to
            // fire on the first occurrence because the series did not
            // exist when the rule started evaluating.
            for outcome in [
                "allow",
                "deny_destination",
                "deny_match_key_missing",
                "unavailable",
                "admin_bypass",
                "error",
            ] {
                let _ = metrics.access_decisions_total.with_label_values(&[outcome]);
            }
            for outcome in [
                "success",
                "http_401",
                "http_403",
                "http_4xx",
                "http_5xx",
                "invalid_response",
                "unreachable",
            ] {
                let _ = metrics.fetch_total.with_label_values(&[outcome]);
            }
            metrics
        };

        Self {
            registry,
            build_info,
            http_requests_total,
            http_request_duration_seconds,
            notifications_total,
            sse_connections_active,
            sse_connections_total,
            sse_unique_users_active,
            sse_events_sent_total,
            sse_stream_errors_total,
            sse_connection_duration_seconds,
            auth_requests_total,
            #[cfg(feature = "ecpds")]
            ecpds,
            unique_users: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Pre-initialise per-stream notification series at zero so alert rules
    /// evaluate against existing series from startup (same rationale as the
    /// ECPDS pre-init in `new`). Call once after the schema is loaded.
    pub fn preinit_notification_series<'a>(&self, event_types: impl IntoIterator<Item = &'a str>) {
        for event_type in event_types {
            for status in ["success", "error"] {
                let _ = self
                    .notifications_total
                    .with_label_values(&[event_type, status]);
            }
        }
    }

    /// Track a user connecting to an SSE endpoint.
    /// Returns a guard that decrements on drop.
    pub fn track_sse_connection(
        &self,
        endpoint: &str,
        event_type: &str,
        username: Option<&str>,
    ) -> SseConnectionGuard {
        self.sse_connections_active
            .with_label_values(&[endpoint, event_type])
            .inc();
        self.sse_connections_total
            .with_label_values(&[endpoint, event_type])
            .inc();

        if let Some(u) = username {
            // Recover from poisoning instead of panicking: the map only holds
            // refcounts, and a panic here (or in Drop, where it would abort
            // the process during unwind) is worse than a skewed gauge.
            let mut users = self
                .unique_users
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let count = users
                .entry(endpoint.to_string())
                .or_default()
                .entry(u.to_string())
                .or_insert(0);
            *count += 1;
            if *count == 1 {
                self.sse_unique_users_active
                    .with_label_values(&[endpoint])
                    .inc();
            }
        }

        SseConnectionGuard {
            metrics: self.clone(),
            endpoint: endpoint.to_string(),
            event_type: event_type.to_string(),
            username: username.map(str::to_string),
            connection_duration: self
                .sse_connection_duration_seconds
                .with_label_values(&[endpoint]),
            opened_at: Instant::now(),
        }
    }
}

/// Pre-labelled per-connection counters for frames delivered on an SSE
/// stream. Cheap to clone into stream-mapping closures; obtained from
/// [`SseConnectionGuard::delivery_metrics`] so the labels always match the
/// connection's gauges.
#[derive(Clone)]
pub struct SseDeliveryMetrics {
    events_sent: IntCounter,
    stream_errors: IntCounter,
}

impl SseDeliveryMetrics {
    pub fn inc_events_sent(&self) {
        self.events_sent.inc();
    }

    pub fn inc_stream_errors(&self) {
        self.stream_errors.inc();
    }
}

/// Decrements SSE connection gauges and observes connection duration when
/// dropped (connection closed/disconnected).
pub struct SseConnectionGuard {
    metrics: AppMetrics,
    endpoint: String,
    event_type: String,
    username: Option<String>,
    connection_duration: Histogram,
    opened_at: Instant,
}

impl SseConnectionGuard {
    /// Counters labelled with this connection's endpoint and event type, for
    /// counting delivered frames inside the stream pipeline.
    pub fn delivery_metrics(&self) -> SseDeliveryMetrics {
        SseDeliveryMetrics {
            events_sent: self
                .metrics
                .sse_events_sent_total
                .with_label_values(&[&self.endpoint, &self.event_type]),
            stream_errors: self
                .metrics
                .sse_stream_errors_total
                .with_label_values(&[&self.endpoint, &self.event_type]),
        }
    }
}

impl Drop for SseConnectionGuard {
    fn drop(&mut self) {
        self.connection_duration
            .observe(self.opened_at.elapsed().as_secs_f64());
        self.metrics
            .sse_connections_active
            .with_label_values(&[&self.endpoint, &self.event_type])
            .dec();

        if let Some(username) = &self.username {
            // See track_sse_connection: poisoning recovery avoids a
            // panic-in-Drop, which would abort the process mid-unwind.
            let mut users = self
                .metrics
                .unique_users
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(endpoint_users) = users.get_mut(&self.endpoint)
                && let Some(count) = endpoint_users.get_mut(username)
            {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    endpoint_users.remove(username);
                    self.metrics
                        .sse_unique_users_active
                        .with_label_values(&[&self.endpoint])
                        .dec();
                }
            }
        }
    }
}

/// Wraps an SSE byte stream, holding the connection guard alive until the
/// stream is dropped (i.e. client disconnects or server shuts down).
pub struct GuardedSseStream<S> {
    #[allow(dead_code)]
    guard: SseConnectionGuard,
    inner: std::pin::Pin<Box<S>>,
}

impl<S> GuardedSseStream<S> {
    pub fn new(inner: std::pin::Pin<Box<S>>, guard: SseConnectionGuard) -> Self {
        Self { guard, inner }
    }
}

impl<S> futures_util::Stream for GuardedSseStream<S>
where
    S: futures_util::Stream,
{
    type Item = S::Item;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}

/// Spawn a lightweight metrics-only HTTP server on the given listener.
///
/// Wraps the same `TracingLogger` + `RequestIdHeader` pair the main server
/// uses, so a `/metrics` scrape (or an ad-hoc `curl -i /metrics` during
/// incident response) carries the same `X-Request-ID` correlation id as
/// every other aviso response.
pub fn run_metrics_server(
    listener: TcpListener,
    registry: Registry,
) -> Result<Server, std::io::Error> {
    let registry = web::Data::new(registry);
    let server = HttpServer::new(move || {
        App::new()
            .wrap(RequestIdHeader)
            .wrap(TracingLogger::<AvisoRootSpanBuilder>::new())
            .app_data(registry.clone())
            .route("/metrics", web::get().to(metrics_handler))
    })
    .listen(listener)?
    .shutdown_timeout(5)
    .disable_signals()
    .run();
    Ok(server)
}

async fn metrics_handler(registry: web::Data<Registry>) -> HttpResponse {
    let encoder = TextEncoder::new();
    let metric_families = registry.gather();
    let mut buffer = Vec::new();
    if encoder.encode(&metric_families, &mut buffer).is_err() {
        return HttpResponse::InternalServerError().finish();
    }
    HttpResponse::Ok()
        .content_type(encoder.format_type())
        .body(buffer)
}

/// Collect default process metrics (CPU, memory, open FDs) when available.
pub fn register_process_metrics(registry: &Registry) {
    #[cfg(target_os = "linux")]
    {
        let pc =
            prometheus::process_collector::ProcessCollector::new(std::process::id() as i32, "");
        let _ = registry.register(Box::new(pc));
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = registry;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gauge_value(metrics: &AppMetrics, name: &str, labels: &[&str]) -> i64 {
        match name {
            "sse_connections_active" => metrics
                .sse_connections_active
                .with_label_values(labels)
                .get(),
            "sse_unique_users_active" => metrics
                .sse_unique_users_active
                .with_label_values(labels)
                .get(),
            _ => panic!("unknown gauge: {name}"),
        }
    }

    fn counter_value(metrics: &AppMetrics, name: &str, labels: &[&str]) -> u64 {
        match name {
            "sse_connections_total" => metrics
                .sse_connections_total
                .with_label_values(labels)
                .get(),
            "notifications_total" => metrics.notifications_total.with_label_values(labels).get(),
            "auth_requests_total" => metrics.auth_requests_total.with_label_values(labels).get(),
            _ => panic!("unknown counter: {name}"),
        }
    }

    #[test]
    fn new_metrics_start_at_zero() {
        let m = AppMetrics::new();
        assert_eq!(
            counter_value(&m, "sse_connections_total", &["watch", "mars"]),
            0
        );
        assert_eq!(
            gauge_value(&m, "sse_connections_active", &["watch", "mars"]),
            0
        );
    }

    #[test]
    fn track_sse_connection_increments_and_guard_drop_decrements() {
        let m = AppMetrics::new();

        let guard = m.track_sse_connection("watch", "mars", None);
        assert_eq!(
            gauge_value(&m, "sse_connections_active", &["watch", "mars"]),
            1
        );
        assert_eq!(
            counter_value(&m, "sse_connections_total", &["watch", "mars"]),
            1
        );

        drop(guard);
        assert_eq!(
            gauge_value(&m, "sse_connections_active", &["watch", "mars"]),
            0
        );
        // Counter stays at 1 after drop.
        assert_eq!(
            counter_value(&m, "sse_connections_total", &["watch", "mars"]),
            1
        );
    }

    #[test]
    fn multiple_connections_stack_on_active_gauge() {
        let m = AppMetrics::new();
        let g1 = m.track_sse_connection("watch", "mars", None);
        let g2 = m.track_sse_connection("watch", "mars", None);
        assert_eq!(
            gauge_value(&m, "sse_connections_active", &["watch", "mars"]),
            2
        );

        drop(g1);
        assert_eq!(
            gauge_value(&m, "sse_connections_active", &["watch", "mars"]),
            1
        );

        drop(g2);
        assert_eq!(
            gauge_value(&m, "sse_connections_active", &["watch", "mars"]),
            0
        );
    }

    #[test]
    fn unique_users_gauge_tracks_distinct_users() {
        let m = AppMetrics::new();

        let g1 = m.track_sse_connection("watch", "mars", Some("alice"));
        assert_eq!(gauge_value(&m, "sse_unique_users_active", &["watch"]), 1);

        // Second connection from same user does not increment unique gauge.
        let g2 = m.track_sse_connection("watch", "mars", Some("alice"));
        assert_eq!(gauge_value(&m, "sse_unique_users_active", &["watch"]), 1);

        // Different user increments unique gauge.
        let g3 = m.track_sse_connection("watch", "mars", Some("bob"));
        assert_eq!(gauge_value(&m, "sse_unique_users_active", &["watch"]), 2);

        // Drop one of alice's connections — still one left, gauge unchanged.
        drop(g1);
        assert_eq!(gauge_value(&m, "sse_unique_users_active", &["watch"]), 2);

        // Drop alice's last connection — gauge decrements.
        drop(g2);
        assert_eq!(gauge_value(&m, "sse_unique_users_active", &["watch"]), 1);

        drop(g3);
        assert_eq!(gauge_value(&m, "sse_unique_users_active", &["watch"]), 0);
    }

    #[test]
    fn anonymous_connections_do_not_affect_unique_users_gauge() {
        let m = AppMetrics::new();
        let guard = m.track_sse_connection("watch", "mars", None);
        assert_eq!(gauge_value(&m, "sse_unique_users_active", &["watch"]), 0);
        drop(guard);
        assert_eq!(gauge_value(&m, "sse_unique_users_active", &["watch"]), 0);
    }

    #[test]
    fn separate_endpoints_track_independently() {
        let m = AppMetrics::new();
        let g1 = m.track_sse_connection("watch", "mars", Some("alice"));
        let g2 = m.track_sse_connection("replay", "mars", Some("alice"));

        assert_eq!(gauge_value(&m, "sse_unique_users_active", &["watch"]), 1);
        assert_eq!(gauge_value(&m, "sse_unique_users_active", &["replay"]), 1);
        assert_eq!(
            gauge_value(&m, "sse_connections_active", &["watch", "mars"]),
            1
        );
        assert_eq!(
            gauge_value(&m, "sse_connections_active", &["replay", "mars"]),
            1
        );

        drop(g1);
        assert_eq!(gauge_value(&m, "sse_unique_users_active", &["watch"]), 0);
        assert_eq!(gauge_value(&m, "sse_unique_users_active", &["replay"]), 1);

        drop(g2);
        assert_eq!(gauge_value(&m, "sse_unique_users_active", &["replay"]), 0);
    }

    #[test]
    fn metrics_handler_returns_prometheus_text() {
        let m = AppMetrics::new();
        m.notifications_total
            .with_label_values(&["mars", "success"])
            .inc();

        let encoder = TextEncoder::new();
        let families = m.registry.gather();
        let mut buf = Vec::new();
        encoder.encode(&families, &mut buf).expect("encode ok");
        let output = String::from_utf8(buf).expect("valid utf8");

        assert!(
            output.contains("aviso_notifications_total"),
            "output should contain metric name"
        );
        assert!(
            output.contains(r#"event_type="mars""#),
            "output should contain label"
        );
    }

    #[test]
    fn guard_drop_observes_connection_duration() {
        let m = AppMetrics::new();
        let histogram = m
            .sse_connection_duration_seconds
            .with_label_values(&["watch"]);

        let guard = m.track_sse_connection("watch", "mars", None);
        assert_eq!(histogram.get_sample_count(), 0);

        drop(guard);
        assert_eq!(histogram.get_sample_count(), 1);
    }

    #[test]
    fn delivery_metrics_increment_counters_with_connection_labels() {
        let m = AppMetrics::new();
        let guard = m.track_sse_connection("replay", "mars", None);

        let delivery = guard.delivery_metrics();
        delivery.inc_events_sent();
        delivery.inc_events_sent();
        delivery.inc_stream_errors();

        assert_eq!(
            m.sse_events_sent_total
                .with_label_values(&["replay", "mars"])
                .get(),
            2
        );
        assert_eq!(
            m.sse_stream_errors_total
                .with_label_values(&["replay", "mars"])
                .get(),
            1
        );
    }

    #[test]
    fn build_info_and_preinitialized_series_appear_in_scrape_at_startup() {
        let m = AppMetrics::new();
        m.preinit_notification_series(["mars"]);

        let encoder = TextEncoder::new();
        let mut buf = Vec::new();
        encoder
            .encode(&m.registry.gather(), &mut buf)
            .expect("encode ok");
        let output = String::from_utf8(buf).expect("valid utf8");

        assert!(
            output.contains(&format!(
                r#"aviso_build_info{{version="{}"}} 1"#,
                env!("CARGO_PKG_VERSION")
            )),
            "build_info should carry the crate version: {output}"
        );
        for series in [
            r#"aviso_auth_requests_total{mode="direct",outcome="unauthorized"} 0"#,
            r#"aviso_auth_requests_total{mode="trusted_proxy",outcome="success"} 0"#,
            r#"aviso_notifications_total{event_type="unknown",status="rejected"} 0"#,
            r#"aviso_notifications_total{event_type="unknown",status="error"} 0"#,
            r#"aviso_notifications_total{event_type="mars",status="success"} 0"#,
            r#"aviso_notifications_total{event_type="mars",status="error"} 0"#,
        ] {
            assert!(
                output.contains(series),
                "series should be pre-initialised at zero: {series}\n{output}"
            );
        }
    }

    #[test]
    fn register_process_metrics_does_not_panic() {
        let registry = Registry::new();
        register_process_metrics(&registry);
        #[cfg(target_os = "linux")]
        {
            let families = registry.gather();
            assert!(
                !families.is_empty(),
                "process metrics should register at least one family"
            );
        }
    }

    #[cfg(feature = "ecpds")]
    #[test]
    fn ecpds_metrics_register_and_publish() {
        let m = AppMetrics::new();
        m.ecpds.cache_hits_total.inc();
        m.ecpds.cache_misses_total.inc();
        m.ecpds.cache_size.set(7);
        m.ecpds
            .access_decisions_total
            .with_label_values(&["allow"])
            .inc();
        m.ecpds
            .access_decisions_total
            .with_label_values(&["deny_destination"])
            .inc();

        let encoder = TextEncoder::new();
        let mut buf = Vec::new();
        encoder
            .encode(&m.registry.gather(), &mut buf)
            .expect("encode ok");
        let output = String::from_utf8(buf).expect("valid utf8");

        assert!(output.contains("aviso_ecpds_cache_hits_total"));
        assert!(output.contains("aviso_ecpds_cache_misses_total"));
        assert!(output.contains("aviso_ecpds_cache_size"));
        assert!(output.contains("aviso_ecpds_access_decisions_total"));
        assert!(output.contains(r#"outcome="allow""#));
        assert!(output.contains(r#"outcome="deny_destination""#));
    }

    // The metrics-only server must wrap the same middleware stack
    // (TracingLogger + RequestIdHeader) as the main server, so an operator
    // running `curl -i /metrics` during incident response receives an
    // X-Request-ID matching the one in server logs. Without these wraps the
    // header is silently absent, which the original PR description glossed
    // over.
    #[actix_web::test]
    async fn metrics_response_carries_x_request_id_header() {
        use actix_web::test::{TestRequest, call_service, init_service};

        let registry = Registry::new();
        let registry_data = web::Data::new(registry);
        let app = init_service(
            App::new()
                .wrap(RequestIdHeader)
                .wrap(TracingLogger::<AvisoRootSpanBuilder>::new())
                .app_data(registry_data)
                .route("/metrics", web::get().to(metrics_handler)),
        )
        .await;

        let res = call_service(&app, TestRequest::get().uri("/metrics").to_request()).await;
        assert_eq!(res.status(), actix_web::http::StatusCode::OK);

        let value = res
            .headers()
            .get("x-request-id")
            .expect("metrics response should carry X-Request-ID")
            .to_str()
            .expect("header should be ascii");
        let uuid_re =
            regex::Regex::new(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")
                .expect("valid uuid regex");
        assert!(
            uuid_re.is_match(value),
            "metrics X-Request-ID should be a canonical UUID, got: {value}"
        );
    }
}
