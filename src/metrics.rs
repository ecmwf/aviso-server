use actix_web::{App, HttpResponse, HttpServer, dev::Server, web};
use prometheus::{
    Encoder, IntCounterVec, IntGaugeVec, Registry, TextEncoder, opts,
    register_int_counter_vec_with_registry, register_int_gauge_vec_with_registry,
};
use std::collections::HashMap;
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

/// Application-level metrics registered in a shared Prometheus registry.
#[derive(Clone, Debug)]
pub struct AppMetrics {
    pub registry: Registry,
    pub notifications_total: IntCounterVec,
    pub sse_connections_active: IntGaugeVec,
    pub sse_connections_total: IntCounterVec,
    pub sse_unique_users_active: IntGaugeVec,
    pub auth_requests_total: IntCounterVec,
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

        let notifications_total = register_int_counter_vec_with_registry!(
            opts!(
                "aviso_notifications_total",
                "Total notification requests by event type and outcome"
            ),
            &["event_type", "status"],
            registry
        )
        .expect("metric must register");

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

        let auth_requests_total = register_int_counter_vec_with_registry!(
            opts!(
                "aviso_auth_requests_total",
                "Authentication attempts by mode and outcome"
            ),
            &["mode", "outcome"],
            registry
        )
        .expect("metric must register");

        Self {
            registry,
            notifications_total,
            sse_connections_active,
            sse_connections_total,
            sse_unique_users_active,
            auth_requests_total,
            unique_users: Arc::new(Mutex::new(HashMap::new())),
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
            let mut users = self.unique_users.lock().expect("metrics lock poisoned");
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
        }
    }
}

/// Decrements SSE connection gauges when dropped (connection closed/disconnected).
pub struct SseConnectionGuard {
    metrics: AppMetrics,
    endpoint: String,
    event_type: String,
    username: Option<String>,
}

impl Drop for SseConnectionGuard {
    fn drop(&mut self) {
        self.metrics
            .sse_connections_active
            .with_label_values(&[&self.endpoint, &self.event_type])
            .dec();

        if let Some(username) = &self.username {
            let mut users = self
                .metrics
                .unique_users
                .lock()
                .expect("metrics lock poisoned");
            if let Some(endpoint_users) = users.get_mut(&self.endpoint)
                && let Some(count) = endpoint_users.get_mut(username)
            {
                *count -= 1;
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
pub fn run_metrics_server(
    listener: TcpListener,
    registry: Registry,
) -> Result<Server, std::io::Error> {
    let registry = web::Data::new(registry);
    let server = HttpServer::new(move || {
        App::new()
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
    let pc = prometheus::process_collector::ProcessCollector::new(std::process::id() as i32, "");
    let _ = registry.register(Box::new(pc));
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
    fn register_process_metrics_does_not_panic() {
        let registry = Registry::new();
        register_process_metrics(&registry);
        // Verify at least one process metric family is present.
        let families = registry.gather();
        assert!(
            !families.is_empty(),
            "process metrics should register at least one family"
        );
    }
}
