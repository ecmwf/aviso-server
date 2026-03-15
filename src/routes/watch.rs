use crate::auth::middleware::get_username;
use crate::error::{
    RequestKind, request_parse_error_response, request_validation_error_response,
    sse_error_response,
};
use crate::handlers::{StreamingRequestProcessor, ValidationConfig, parse_and_validate_request};
use crate::metrics::AppMetrics;
use crate::notification::decode_subject_for_display;
use crate::notification_backend::NotificationBackend;
use crate::notification_backend::replay::StartAt;
use crate::routes::streaming::{StreamOperation, enforce_stream_auth, record_start_at_span_fields};
use crate::sse::{create_historical_then_live_stream, create_watch_sse_stream};
use crate::telemetry::{SERVICE_NAME, SERVICE_VERSION};
use crate::types::NotificationRequest;
use actix_web::{HttpRequest, HttpResponse, web};
use std::sync::Arc;
use std::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::info;
use tracing_actix_web::RequestId;

/// Watch endpoint handler with SSE streaming
///
/// Processes watch requests and establishes SSE streaming for real-time notifications.
/// Validates request parameters and sets up live notification streaming with optional
/// historical replay functionality when from_id or from_date parameters are provided.
/// Applies spatial and field filtering to ensure only matching notifications are streamed.
#[utoipa::path(
    post,
    path = "/api/v1/watch",
    tag = "streaming",
    request_body = NotificationRequest,
    responses(
        (status = 200, description = "SSE stream established successfully", content_type = "text/event-stream"),
        (status = 400, description = "Invalid request parameters"),
        (status = 401, description = "Missing or invalid credentials (when stream requires auth)"),
        (status = 403, description = "Valid credentials but insufficient roles for this stream"),
        (status = 500, description = "Failed to establish stream"),
        (status = 503, description = "Authentication service unavailable (direct mode)")
    ),
    security(
        ("bearer_jwt" = []),
        ("basic" = []),
    )
)]
#[tracing::instrument(
    skip(notification_backend, shutdown, metrics),
    fields(
        event_type = tracing::field::Empty,
        request_id = %request_id,
        from_id = tracing::field::Empty,
        from_date = tracing::field::Empty,
        endpoint = "watch",
    )
)]
pub async fn watch(
    body: web::Bytes,
    http_request: HttpRequest,
    notification_backend: web::Data<Arc<dyn NotificationBackend>>,
    shutdown: web::Data<CancellationToken>,
    request_id: RequestId,
    metrics: Option<web::Data<AppMetrics>>,
) -> HttpResponse {
    // Parse and validate request structure
    let notification_request = match parse_and_validate_request(&body) {
        Ok(req) => req,
        Err(e) => return request_parse_error_response(RequestKind::Watch, e),
    };

    // Enforce schema-level auth before stream setup to fail fast.
    if let Err(response) = enforce_stream_auth(
        &http_request,
        &notification_request.event_type,
        StreamOperation::Read,
    ) {
        return response;
    }

    // Process request using shared processor
    let context = match StreamingRequestProcessor::process_request(
        &notification_request,
        request_id,
        ValidationConfig::for_watch(),
    ) {
        Ok(ctx) => ctx,
        Err(e) => return request_validation_error_response(RequestKind::Watch, e),
    };

    // Update tracing context
    tracing::Span::current().record("event_type", &context.event_type);
    record_start_at_span_fields(context.start_at);

    // Use canonicalized filtering parameters produced by request processing.
    let filtering_params = Arc::new(context.canonicalized_params.clone());
    let filtering_constraints = Arc::new(context.identifier_constraints.clone());

    // Guard is created before stream setup so it can be moved into the SSE
    // response body. On setup failure the guard drops immediately, causing a
    // brief +1/-1 on the active gauge — acceptable for production metrics.
    let sse_guard = metrics.as_ref().map(|m| {
        let username = get_username(&http_request);
        m.track_sse_connection("watch", &context.event_type, username.as_deref())
    });

    // Determine streaming mode and create appropriate stream
    let display_topic = decode_subject_for_display(&context.topic);
    let setup_started_at = Instant::now();
    let (stream_mode, sse_response) = if !matches!(context.start_at, StartAt::LiveOnly) {
        (
            "historical_then_live",
            create_historical_then_live_stream(
                context.topic.clone(),
                notification_backend.get_ref().clone(),
                context.start_at,
                shutdown.clone(),
                filtering_params.clone(),
                filtering_constraints.clone(),
                sse_guard,
            )
            .await,
        )
    } else {
        (
            "live_only",
            create_watch_sse_stream(
                context.topic.clone(),
                notification_backend.get_ref().clone(),
                shutdown.clone(),
                filtering_params.clone(),
                filtering_constraints.clone(),
                sse_guard,
            )
            .await,
        )
    };

    match sse_response {
        Ok(response) => {
            info!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_name = "api.watch.stream.established",
                outcome = "success",
                topic = %display_topic,
                start_at = ?context.start_at,
                stream_mode = stream_mode,
                setup_duration_ms = setup_started_at.elapsed().as_millis() as u64,
                "SSE stream established successfully"
            );
            response
        }
        Err(e) => sse_error_response(e, &context.topic, &context.request_id.to_string()),
    }
}
