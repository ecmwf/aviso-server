use crate::error::{sse_error_response, validation_error_response};
use crate::handlers::{StreamingRequestProcessor, ValidationConfig};
use crate::notification_backend::NotificationBackend;
use crate::sse::create_replay_only_stream;
use crate::types::NotificationRequest;
use actix_web::{HttpResponse, web};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing::info;
use tracing_actix_web::RequestId;

/// Replay endpoint handler for historical message streaming
///
/// Processes replay requests and establishes SSE streaming for historical notifications only.
/// Validates request parameters and sets up replay-only streaming that terminates after
/// historical data is delivered.
#[tracing::instrument(
    skip(notification_request, notification_backend, shutdown),
    fields(
        event_type = tracing::field::Empty,
        request_id = %request_id,
        from_id = tracing::field::Empty,
        from_date = tracing::field::Empty,
        endpoint = "replay",
    )
)]
pub async fn replay(
    notification_request: web::Json<NotificationRequest>,
    notification_backend: web::Data<Arc<dyn NotificationBackend>>,
    shutdown: web::Data<CancellationToken>,
    request_id: RequestId,
) -> HttpResponse {
    // Process request using shared processor
    let context = match StreamingRequestProcessor::process_request(
        &notification_request,
        request_id,
        ValidationConfig::for_replay(),
    ) {
        Ok(ctx) => ctx,
        Err(e) => return validation_error_response("Replay Request", e),
    };

    // Update tracing context
    tracing::Span::current().record("event_type", &context.event_type);
    if let Some(id) = context.from_id {
        tracing::Span::current().record("from_id", id);
    }
    if let Some(date) = &context.from_date {
        tracing::Span::current().record("from_date", date.to_rfc3339());
    }

    info!(
        topic = %context.topic,
        from_id = ?context.from_id,
        from_date = ?context.from_date,
        "Starting replay-only SSE stream"
    );

    // Create replay-only stream
    match create_replay_only_stream(
        context.topic.clone(),
        notification_backend.get_ref().clone(),
        context.from_id,
        context.from_date,
        shutdown.clone(),
    )
    .await
    {
        Ok(response) => {
            info!(topic = %context.topic, "Replay-only SSE stream established successfully");
            response
        }
        Err(e) => sse_error_response(e, &context.topic, &context.request_id.to_string()),
    }
}
