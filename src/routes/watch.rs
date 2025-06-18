use crate::configuration::Settings;
use crate::error::{sse_error_response, validation_error_response};
use crate::notification::{NotificationHandler, OperationType};
use crate::notification_backend::NotificationBackend;
use crate::sse::{create_historical_then_live_stream, create_watch_sse_stream};
use crate::types::NotificationRequest;
use actix_web::{HttpResponse, web};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use tracing_actix_web::RequestId;

/// Watch endpoint handler with SSE streaming
///
/// Processes watch requests and establishes SSE streaming for real-time notifications.
/// Validates request parameters and sets up live notification streaming with optional
/// historical replay functionality when from_id or from_date parameters are provided.
#[tracing::instrument(
    skip(notification_request, notification_backend),
    fields(
        event_type = tracing::field::Empty,
        request_id = %request_id,
        from_id = tracing::field::Empty,
        from_date = tracing::field::Empty,
        replay_mode = tracing::field::Empty,
    )
)]
pub async fn watch(
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

    // Validate watch-specific parameters (from_id and from_date)
    let (from_id, from_date) = match notification_request.validate_watch_parameters() {
        Ok(params) => params,
        Err(e) => {
            return validation_error_response("Watch Parameters", e);
        }
    };

    // Determine if this is a replay request
    let is_replay_mode = from_id.is_some() || from_date.is_some();

    // Update tracing context with validated parameters
    if let Some(id) = from_id {
        tracing::Span::current().record("from_id", id);
    }
    if let Some(date) = &from_date {
        tracing::Span::current().record("from_date", date.to_rfc3339());
    }
    tracing::Span::current().record("replay_mode", is_replay_mode);

    // Process watch request with only required fields (watch operation)
    let notification_handler =
        NotificationHandler::from_config(Settings::get_global_notification_schema().as_ref());

    let notification_result = match notification_handler.process_request(
        event_type,
        request_params,
        OperationType::Watch,
    ) {
        Ok(result) => result,
        Err(e) => return validation_error_response("Watch", e),
    };

    // Log the processing decision based on mode
    if is_replay_mode {
        info!(
            event_type = %event_type,
            topic = %notification_result.topic,
            param_count = request_params.len(),
            from_id = ?from_id,
            from_date = ?from_date,
            "Starting SSE stream with historical replay + live notifications"
        );
    } else {
        info!(
            event_type = %event_type,
            topic = %notification_result.topic,
            param_count = request_params.len(),
            "Starting SSE stream for live notifications only"
        );
    }

    // Create SSE stream based on whether replay is requested
    let sse_response = if is_replay_mode {
        // Historical replay + live-streaming mode
        create_historical_then_live_stream(
            notification_result.topic.clone(),
            notification_backend.get_ref().clone(),
            from_id,
            from_date,
            shutdown.clone(),
        )
        .await
    } else {
        // Live-only streaming mode (existing functionality)
        create_watch_sse_stream(
            notification_result.topic.clone(),
            notification_backend.get_ref().clone(),
            shutdown.clone(),
        )
        .await
    };

    // Handle the SSE response
    match sse_response {
        Ok(sse_response) => {
            info!(
                topic = %notification_result.topic,
                replay_mode = is_replay_mode,
                "SSE stream established successfully"
            );
            sse_response
        }
        Err(e) => {
            warn!(
                error = %e,
                topic = %notification_result.topic,
                replay_mode = is_replay_mode,
                "Failed to create SSE stream"
            );
            // Use your existing consistent error helper
            sse_error_response(e, &notification_result.topic, &request_id.to_string())
        }
    }
}
