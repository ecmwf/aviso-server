// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Per-request RED metrics (`aviso_http_requests_total`,
//! `aviso_http_request_duration_seconds`, `aviso_http_requests_in_flight`)
//! for every route on the main server.
//!
//! The `route` label is the *matched route pattern* (e.g.
//! `/api/v1/schema/{event_type}`), never the raw request path, so label
//! cardinality stays bounded under path scans: unrouted requests collapse
//! into `route="unmatched"`. (It is named `route`, not `endpoint`, to avoid
//! colliding with the Prometheus Operator target label `endpoint`.) Durations
//! are measured until response headers are ready, which for SSE routes means
//! stream *setup* latency rather than connection lifetime.
//!
//! The in-flight gauge is labelled by method only: the route pattern is not
//! known until routing completes, by which point the request is already in
//! flight, so there is no correct route value to use at increment time.
//!
//! Reads [`AppMetrics`] from app data and is a pure passthrough when metrics
//! are disabled, mirroring how route handlers treat
//! `Option<web::Data<AppMetrics>>`.

use std::borrow::Cow;
use std::future::{Ready, ready};
use std::rc::Rc;
use std::time::Instant;

use actix_web::{
    Error,
    dev::{Service, ServiceRequest, ServiceResponse, Transform, forward_ready},
    http::Method,
    web,
};
use futures_util::future::LocalBoxFuture;
use prometheus::IntGauge;

use crate::metrics::AppMetrics;

/// Decrements the in-flight gauge when the request future completes or is
/// dropped (client disconnect / cancellation), so the gauge cannot leak.
struct InFlightGuard(IntGauge);

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.0.dec();
    }
}

/// Bound the method label to well-known HTTP methods; anything else (HTTP
/// allows arbitrary extension tokens) is collapsed to keep cardinality fixed.
fn method_label(method: &Method) -> &'static str {
    match method.as_str() {
        "GET" => "GET",
        "POST" => "POST",
        "PUT" => "PUT",
        "DELETE" => "DELETE",
        "PATCH" => "PATCH",
        "HEAD" => "HEAD",
        "OPTIONS" => "OPTIONS",
        "CONNECT" => "CONNECT",
        "TRACE" => "TRACE",
        _ => "other",
    }
}

/// Middleware recording request count and duration per matched route.
///
/// Register inside `RequestIdHeader`/`TracingLogger` so the recorded
/// duration covers routing, scoped auth middleware, and the handler:
///
/// ```ignore
/// App::new()
///     .wrap(HttpMetrics)
///     .wrap(RequestIdHeader)
///     .wrap(TracingLogger::<AvisoRootSpanBuilder>::new())
/// ```
#[derive(Default, Clone)]
pub struct HttpMetrics;

impl<S, B> Transform<S, ServiceRequest> for HttpMetrics
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Transform = HttpMetricsMiddleware<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(HttpMetricsMiddleware {
            service: Rc::new(service),
        }))
    }
}

pub struct HttpMetricsMiddleware<S> {
    service: Rc<S>,
}

impl<S, B> Service<ServiceRequest> for HttpMetricsMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let service = Rc::clone(&self.service);
        // Capture before the call: `req` is consumed, and on the Err path no
        // request is available afterwards.
        let metrics = req.app_data::<web::Data<AppMetrics>>().cloned();
        let method = method_label(req.method());
        let started_at = Instant::now();

        Box::pin(async move {
            // Increment in-flight on entry; the guard decrements on drop,
            // covering the Ok, Err, and cancellation (client disconnect) paths.
            let _in_flight = metrics.as_ref().map(|m| {
                let gauge = m.http_requests_in_flight.with_label_values(&[method]);
                gauge.inc();
                InFlightGuard(gauge)
            });

            let result = service.call(req).await;

            if let Some(m) = &metrics {
                // Route matching happens inside the inner service, so the
                // pattern is only available on the response's request.
                let (route, status_code): (Cow<'static, str>, _) = match &result {
                    Ok(res) => (
                        res.request()
                            .match_pattern()
                            .map_or(Cow::Borrowed("unmatched"), Cow::Owned),
                        res.status(),
                    ),
                    Err(e) => (Cow::Borrowed("error"), e.as_response_error().status_code()),
                };

                m.http_requests_total
                    .with_label_values(&[route.as_ref(), method, status_code.as_str()])
                    .inc();
                m.http_request_duration_seconds
                    .with_label_values(&[route.as_ref(), method])
                    .observe(started_at.elapsed().as_secs_f64());
            }

            result
        })
    }
}

#[cfg(test)]
mod tests {
    use super::HttpMetrics;
    use crate::metrics::AppMetrics;
    use actix_web::{
        App, HttpResponse,
        http::StatusCode,
        test::{TestRequest, call_service, init_service},
        web,
    };

    async fn ok_handler() -> HttpResponse {
        HttpResponse::Ok().body("ok")
    }

    async fn item_handler(path: web::Path<String>) -> HttpResponse {
        HttpResponse::Ok().body(path.into_inner())
    }

    async fn fail_handler() -> HttpResponse {
        HttpResponse::InternalServerError().finish()
    }

    fn requests_count(m: &AppMetrics, route: &str, method: &str, status: &str) -> u64 {
        m.http_requests_total
            .with_label_values(&[route, method, status])
            .get()
    }

    fn duration_sample_count(m: &AppMetrics, route: &str, method: &str) -> u64 {
        m.http_request_duration_seconds
            .with_label_values(&[route, method])
            .get_sample_count()
    }

    #[actix_web::test]
    async fn records_count_and_duration_for_matched_route() {
        let metrics = AppMetrics::new();
        let app = init_service(
            App::new()
                .wrap(HttpMetrics)
                .app_data(web::Data::new(metrics.clone()))
                .route("/ok", web::get().to(ok_handler)),
        )
        .await;

        let res = call_service(&app, TestRequest::get().uri("/ok").to_request()).await;
        assert_eq!(res.status(), StatusCode::OK);

        assert_eq!(requests_count(&metrics, "/ok", "GET", "200"), 1);
        assert_eq!(duration_sample_count(&metrics, "/ok", "GET"), 1);
    }

    #[actix_web::test]
    async fn endpoint_label_is_route_pattern_not_raw_path() {
        let metrics = AppMetrics::new();
        let app = init_service(
            App::new()
                .wrap(HttpMetrics)
                .app_data(web::Data::new(metrics.clone()))
                .route("/items/{id}", web::get().to(item_handler)),
        )
        .await;

        for uri in ["/items/1", "/items/2", "/items/abc"] {
            call_service(&app, TestRequest::get().uri(uri).to_request()).await;
        }

        assert_eq!(requests_count(&metrics, "/items/{id}", "GET", "200"), 3);
        assert_eq!(requests_count(&metrics, "/items/1", "GET", "200"), 0);
    }

    #[actix_web::test]
    async fn unrouted_requests_collapse_into_unmatched_endpoint() {
        let metrics = AppMetrics::new();
        let app = init_service(
            App::new()
                .wrap(HttpMetrics)
                .app_data(web::Data::new(metrics.clone()))
                .route("/ok", web::get().to(ok_handler)),
        )
        .await;

        for uri in ["/nope", "/admin.php", "/.env"] {
            let res = call_service(&app, TestRequest::get().uri(uri).to_request()).await;
            assert_eq!(res.status(), StatusCode::NOT_FOUND);
        }

        assert_eq!(requests_count(&metrics, "unmatched", "GET", "404"), 3);
    }

    #[actix_web::test]
    async fn error_status_is_recorded() {
        let metrics = AppMetrics::new();
        let app = init_service(
            App::new()
                .wrap(HttpMetrics)
                .app_data(web::Data::new(metrics.clone()))
                .route("/boom", web::get().to(fail_handler)),
        )
        .await;

        let res = call_service(&app, TestRequest::get().uri("/boom").to_request()).await;
        assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);

        assert_eq!(requests_count(&metrics, "/boom", "GET", "500"), 1);
    }

    #[actix_web::test]
    async fn inner_service_err_records_error_endpoint_with_derived_status() {
        use actix_web::body::BoxBody;
        use actix_web::dev::{Service, ServiceRequest, ServiceResponse, Transform};
        use std::future::{Ready, ready};
        use std::task::{Context, Poll};

        struct AlwaysErr;

        impl Service<ServiceRequest> for AlwaysErr {
            type Response = ServiceResponse<BoxBody>;
            type Error = actix_web::Error;
            type Future = Ready<Result<Self::Response, Self::Error>>;

            fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
                Poll::Ready(Ok(()))
            }

            fn call(&self, _req: ServiceRequest) -> Self::Future {
                ready(Err(actix_web::error::ErrorImATeapot("boom")))
            }
        }

        let metrics = AppMetrics::new();
        let mw = HttpMetrics
            .new_transform(AlwaysErr)
            .await
            .expect("transform must build");

        let req = TestRequest::get()
            .uri("/whatever")
            .app_data(web::Data::new(metrics.clone()))
            .to_srv_request();
        let result = mw.call(req).await;

        assert!(result.is_err(), "error must propagate unchanged");
        assert_eq!(requests_count(&metrics, "error", "GET", "418"), 1);
        assert_eq!(duration_sample_count(&metrics, "error", "GET"), 1);
    }

    #[actix_web::test]
    async fn in_flight_gauge_rises_during_request_and_falls_after() {
        let metrics = AppMetrics::new();

        // Handler observes the in-flight gauge from inside the request, where
        // it must read 1 (this request), then the gauge must return to 0 once
        // the response future completes and the guard drops.
        async fn observing_handler(m: web::Data<AppMetrics>) -> HttpResponse {
            let in_flight = m.http_requests_in_flight.with_label_values(&["GET"]).get();
            HttpResponse::Ok().body(in_flight.to_string())
        }

        let app = init_service(
            App::new()
                .wrap(HttpMetrics)
                .app_data(web::Data::new(metrics.clone()))
                .route("/obs", web::get().to(observing_handler)),
        )
        .await;

        let res = call_service(&app, TestRequest::get().uri("/obs").to_request()).await;
        assert_eq!(res.status(), StatusCode::OK);
        let body = actix_web::body::to_bytes(res.into_body())
            .await
            .expect("body");
        assert_eq!(
            body, "1",
            "gauge should read 1 while the request is in flight"
        );

        assert_eq!(
            metrics
                .http_requests_in_flight
                .with_label_values(&["GET"])
                .get(),
            0,
            "gauge should return to 0 after the request completes",
        );
    }

    #[actix_web::test]
    async fn passthrough_when_metrics_are_absent() {
        let app = init_service(
            App::new()
                .wrap(HttpMetrics)
                .route("/ok", web::get().to(ok_handler)),
        )
        .await;

        let res = call_service(&app, TestRequest::get().uri("/ok").to_request()).await;
        assert_eq!(res.status(), StatusCode::OK);
    }
}
