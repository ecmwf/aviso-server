use crate::auth::middleware::get_username;
use crate::error::{
    RequestKind, request_parse_error_response, request_validation_error_response,
    sse_error_response,
};
use crate::handlers::{StreamingRequestProcessor, ValidationConfig, parse_and_validate_request};
use crate::metrics::AppMetrics;
use crate::notification::decode_subject_for_display;
use crate::notification_backend::NotificationBackend;
use crate::routes::streaming::{StreamOperation, enforce_stream_auth, record_start_at_span_fields};
use crate::sse::create_replay_only_stream;
use crate::telemetry::{SERVICE_NAME, SERVICE_VERSION};
use actix_web::{HttpRequest, HttpResponse, web};
use std::sync::Arc;
use std::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::info;
use tracing_actix_web::RequestId;

/// Replay endpoint handler for historical message streaming
#[utoipa::path(
    post,
    path = "/api/v1/replay",
    tag = "streaming",
    request_body = crate::types::request::NotificationRequest,
    responses(
        (status = 200, description = "Historical replay stream established successfully", content_type = "text/event-stream"),
        (status = 400, description = "Invalid request parameters or missing from_id/from_date"),
        (status = 401, description = "Missing or invalid credentials (when stream requires auth)"),
        (status = 403, description = "Valid credentials but insufficient roles for this stream"),
        (status = 500, description = "Failed to establish replay stream"),
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
        endpoint = "replay",
    )
)]
pub async fn replay(
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
        Err(e) => return request_parse_error_response(RequestKind::Replay, e),
    };

    // Enforce schema-level auth before replay setup to fail fast.
    if let Err(response) = enforce_stream_auth(
        &http_request,
        &notification_request.event_type,
        StreamOperation::Read,
    ) {
        return response;
    }

    let context = match StreamingRequestProcessor::process_request(
        &notification_request,
        request_id,
        ValidationConfig::for_replay(),
    ) {
        Ok(ctx) => ctx,
        Err(e) => return request_validation_error_response(RequestKind::Replay, e),
    };

    tracing::Span::current().record("event_type", &context.event_type);
    record_start_at_span_fields(context.start_at);

    let display_topic = decode_subject_for_display(&context.topic);
    let setup_started_at = Instant::now();

    // Pass canonicalized params for downstream filtering
    let filtering_params = Arc::new(context.canonicalized_params.clone());
    let filtering_constraints = Arc::new(context.identifier_constraints.clone());

    // See watch.rs for why the guard is created before stream setup.
    let sse_guard = metrics.as_ref().map(|m| {
        let username = get_username(&http_request);
        m.track_sse_connection("replay", &context.event_type, username.as_deref())
    });

    match create_replay_only_stream(
        context.topic.clone(),
        notification_backend.get_ref().clone(),
        context.start_at,
        shutdown.clone(),
        filtering_params,
        filtering_constraints,
        sse_guard,
    )
    .await
    {
        Ok(response) => {
            info!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_name = "api.replay.stream.established",
                outcome = "success",
                topic = %display_topic,
                start_at = ?context.start_at,
                stream_mode = "replay_only",
                setup_duration_ms = setup_started_at.elapsed().as_millis() as u64,
                "Replay-only SSE stream established successfully"
            );
            response
        }
        Err(e) => sse_error_response(e, &context.topic, &context.request_id.to_string()),
    }
}
