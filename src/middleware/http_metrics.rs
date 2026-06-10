// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Per-request RED metrics (`aviso_http_requests_total`,
//! `aviso_http_request_duration_seconds`) for every route on the main server.
//!
//! The endpoint label is the *matched route pattern* (e.g.
//! `/api/v1/schema/{event_type}`), never the raw request path, so label
//! cardinality stays bounded under path scans: unrouted requests collapse
//! into `endpoint="unmatched"`. Durations are measured until response
//! headers are ready, which for SSE endpoints means stream *setup* latency
//! rather than connection lifetime.
//!
//! Reads [`AppMetrics`] from app data and is a pure passthrough when metrics
//! are disabled, mirroring how route handlers treat
//! `Option<web::Data<AppMetrics>>`.

use std::future::{Ready, ready};
use std::rc::Rc;
use std::time::Instant;

use actix_web::{
    Error, web,
    dev::{Service, ServiceRequest, ServiceResponse, Transform, forward_ready},
    http::Method,
};
use futures_util::future::LocalBoxFuture;

use crate::metrics::AppMetrics;

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
            let result = service.call(req).await;

            if let Some(m) = metrics {
                // Route matching happens inside the inner service, so the
                // pattern is only available on the response's request.
                let (endpoint, status_code) = match &result {
                    Ok(res) => (
                        res.request()
                            .match_pattern()
                            .unwrap_or_else(|| "unmatched".to_string()),
                        res.status(),
                    ),
                    Err(e) => ("error".to_string(), e.as_response_error().status_code()),
                };

                m.http_requests_total
                    .with_label_values(&[&endpoint, method, status_code.as_str()])
                    .inc();
                m.http_request_duration_seconds
                    .with_label_values(&[&endpoint, method])
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

    fn requests_count(m: &AppMetrics, endpoint: &str, method: &str, status: &str) -> u64 {
        m.http_requests_total
            .with_label_values(&[endpoint, method, status])
            .get()
    }

    fn duration_sample_count(m: &AppMetrics, endpoint: &str, method: &str) -> u64 {
        m.http_request_duration_seconds
            .with_label_values(&[endpoint, method])
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
