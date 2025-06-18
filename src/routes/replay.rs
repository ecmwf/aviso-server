//! Replay endpoint handler for historical message retrieval
//!
//! This endpoint provides replay-only functionality that streams historical
//! messages and then closes the connection, unlike the watch endpoint which
//! transitions to live streaming after replay.

use crate::configuration::Settings;
use crate::error::{sse_error_response, validation_error_response};
use crate::notification::{NotificationHandler, OperationType};
use crate::notification_backend::NotificationBackend;
use crate::sse::create_replay_only_stream;
use crate::types::NotificationRequest;
use actix_web::{HttpResponse, web};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use tracing_actix_web::RequestId;

/// Replay endpoint handler for historical message streaming
///
/// Processes replay requests and establishes SSE streaming for historical notifications only.
/// Validates request parameters and sets up replay-only streaming that terminates after
/// historical data is delivered.
///
/// Unlike the watch endpoint, this endpoint:
/// - Requires either from_id or from_date parameter (no live-only mode)
/// - Streams historical messages and then closes the connection
/// - Does not include heartbeats (short-lived connection)
/// - Does not transition to live-streaming
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
    // Extract event type and request parameters from notification_request
    let event_type = &notification_request.event_type;
    let request_params = &notification_request.request;

    // Update tracing context with event type
    tracing::Span::current().record("event_type", event_type);

    // Validate that either from_id or from_date is provided (required for replay)
    let (from_id, from_date) = match notification_request.validate_watch_parameters() {
        Ok((Some(id), None)) => (Some(id), None),
        Ok((None, Some(date))) => (None, Some(date)),
        Ok((Some(_), Some(_))) => {
            // This case should never happen due to validation in validate_watch_parameters()
            // but we need to handle it for exhaustive pattern matching
            return validation_error_response(
                "Replay Parameters",
                anyhow::anyhow!(
                    "Internal error: both from_id and from_date were validated despite strict validation"
                ),
            );
        }
        Ok((None, None)) => {
            return validation_error_response(
                "Replay Parameters",
                anyhow::anyhow!("Replay endpoint requires either from_id or from_date parameter"),
            );
        }
        Err(e) => return validation_error_response("Replay Parameters", e),
    };

    // Update tracing context with validated parameters
    if let Some(id) = from_id {
        tracing::Span::current().record("from_id", id);
    }
    if let Some(date) = &from_date {
        tracing::Span::current().record("from_date", date.to_rfc3339());
    }

    // Process replay request with only required fields (replay operation)
    let notification_handler =
        NotificationHandler::from_config(Settings::get_global_notification_schema().as_ref());

    let notification_result = match notification_handler.process_request(
        event_type,
        request_params,
        OperationType::Replay,
    ) {
        Ok(result) => result,
        Err(e) => {
            warn!(
                error = %e,
                event_type = %event_type,
                request_id = %request_id,
                "Replay request processing failed"
            );
            return validation_error_response("Replay", e);
        }
    };

    info!(
        event_type = %event_type,
        topic = %notification_result.topic,
        param_count = request_params.len(),
        from_id = ?from_id,
        from_date = ?from_date,
        request_id = %request_id,
        "Starting replay-only SSE stream"
    );

    // Create replay-only SSE stream
    match create_replay_only_stream(
        notification_result.topic.clone(),
        notification_backend.get_ref().clone(),
        from_id,
        from_date,
        shutdown.clone(),
    )
    .await
    {
        Ok(sse_response) => {
            info!(
                topic = %notification_result.topic,
                request_id = %request_id,
                "Replay-only SSE stream established successfully"
            );
            sse_response
        }
        Err(e) => {
            warn!(
                error = %e,
                topic = %notification_result.topic,
                request_id = %request_id,
                "Failed to create replay-only SSE stream"
            );
            sse_error_response(e, &notification_result.topic, &request_id.to_string())
        }
    }
}
