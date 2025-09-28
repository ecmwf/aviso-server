use crate::error::{sse_error_response, validation_error_response};
use crate::handlers::{StreamingRequestProcessor, ValidationConfig, parse_and_validate_request};
use crate::notification_backend::NotificationBackend;
use crate::sse::{create_historical_then_live_stream, create_watch_sse_stream};
use crate::types::NotificationRequest;
use actix_web::{HttpResponse, web};
use std::sync::Arc;
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
        (status = 500, description = "Failed to establish stream")
    )
)]
#[tracing::instrument(
    skip(notification_backend, shutdown),
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
    notification_backend: web::Data<Arc<dyn NotificationBackend>>,
    shutdown: web::Data<CancellationToken>,
    request_id: RequestId,
) -> HttpResponse {
    // Parse and validate request structure
    let notification_request = match parse_and_validate_request(&body) {
        Ok(req) => req,
        Err(e) => return validation_error_response("Watch Request", e),
    };
    // Process request using shared processor
    let context = match StreamingRequestProcessor::process_request(
        &notification_request,
        request_id,
        ValidationConfig::for_watch(),
    ) {
        Ok(ctx) => ctx,
        Err(e) => return validation_error_response("Watch Request", e),
    };

    // Update tracing context
    tracing::Span::current().record("event_type", &context.event_type);
    if let Some(id) = context.from_id {
        tracing::Span::current().record("from_id", id);
    }
    if let Some(date) = &context.from_date {
        tracing::Span::current().record("from_date", date.to_rfc3339());
    }

    // Prepare filtering parameters - use ORIGINAL request parameters for spatial filtering
    let original_request_params = Arc::new(notification_request.identifier.clone());

    // Determine streaming mode and create appropriate stream
    let sse_response = if context.from_id.is_some() || context.from_date.is_some() {
        info!(
            topic = %context.topic,
            from_id = ?context.from_id,
            from_date = ?context.from_date,
            "Creating historical replay + live stream"
        );

        create_historical_then_live_stream(
            context.topic.clone(),
            notification_backend.get_ref().clone(),
            context.from_id,
            context.from_date,
            shutdown.clone(),
            original_request_params.clone(),
        )
        .await
    } else {
        info!(topic = %context.topic, "Creating live-only stream");

        create_watch_sse_stream(
            context.topic.clone(),
            notification_backend.get_ref().clone(),
            shutdown.clone(),
            original_request_params.clone(),
        )
        .await
    };

    match sse_response {
        Ok(response) => {
            info!(topic = %context.topic, "SSE stream established successfully");
            response
        }
        Err(e) => sse_error_response(e, &context.topic, &context.request_id.to_string()),
    }
}
