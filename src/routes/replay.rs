use crate::error::{sse_error_response, validation_error_response};
use crate::handlers::{StreamingRequestProcessor, ValidationConfig, parse_and_validate_request};
use crate::notification::decode_subject_for_display;
use crate::notification_backend::NotificationBackend;
use crate::routes::streaming::record_start_at_span_fields;
use crate::sse::create_replay_only_stream;
use actix_web::{HttpResponse, web};
use std::sync::Arc;
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
        (status = 500, description = "Failed to establish replay stream")
    )
)]
#[tracing::instrument(
    skip(notification_backend, shutdown),
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
    notification_backend: web::Data<Arc<dyn NotificationBackend>>,
    shutdown: web::Data<CancellationToken>,
    request_id: RequestId,
) -> HttpResponse {
    // Parse and validate request structure
    let notification_request = match parse_and_validate_request(&body) {
        Ok(req) => req,
        Err(e) => return validation_error_response("Replay Request", e),
    };
    let context = match StreamingRequestProcessor::process_request(
        &notification_request,
        request_id,
        ValidationConfig::for_replay(),
    ) {
        Ok(ctx) => ctx,
        Err(e) => return validation_error_response("Replay Request", e),
    };

    tracing::Span::current().record("event_type", &context.event_type);
    record_start_at_span_fields(context.start_at);

    let display_topic = decode_subject_for_display(&context.topic);
    info!(
        topic = %display_topic,
        start_at = ?context.start_at,
        "Starting replay-only SSE stream"
    );

    // Pass canonicalized params for downstream filtering
    let filtering_params = Arc::new(context.canonicalized_params.clone());

    match create_replay_only_stream(
        context.topic.clone(),
        notification_backend.get_ref().clone(),
        context.start_at,
        shutdown.clone(),
        filtering_params,
    )
    .await
    {
        Ok(response) => {
            info!(topic = %display_topic, "Replay-only SSE stream established successfully");
            response
        }
        Err(e) => sse_error_response(e, &context.topic, &context.request_id.to_string()),
    }
}
