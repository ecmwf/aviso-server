use crate::error::processing_error_response;
use crate::error::validation_error_response;
use crate::handlers::{
    convert_payload_to_string, get_payload_type_name, parse_and_validate_request,
    process_notification_request, save_to_backend,
};
use crate::notification::OperationType;
use crate::notification_backend::NotificationBackend;
use crate::types::NotificationResponse;
use actix_web::{HttpResponse, web};
use std::sync::Arc;
use tracing::{error, info};
use tracing_actix_web::RequestId;

/// Notification endpoint handler
///
/// Processes notification requests with all schema fields required.
/// Validates request format, payload type, processes notification, and saves to backend.
/// Now supports spatial metadata extraction for polygon fields.
#[tracing::instrument(
    skip(body, notification_backend),
    fields(
        event_type = tracing::field::Empty,
        topic = tracing::field::Empty,
        request_id = %request_id,
        spatial_enabled = tracing::field::Empty,
    )
)]
pub async fn notify(
    body: web::Bytes,
    notification_backend: web::Data<Arc<dyn NotificationBackend>>,
    request_id: RequestId,
) -> HttpResponse {
    // Parse and validate request structure
    let payload = match parse_and_validate_request(&body) {
        Ok(p) => p,
        Err(e) => return validation_error_response("Request Validation", e),
    };

    let event_type = &payload.event_type;
    let request_params = &payload.request;

    tracing::Span::current().record("event_type", event_type);

    // Convert PayloadType to serde_json::Value
    let payload_value = payload.payload.as_ref().map(|p| p.to_json_value());

    // Process notification request with payload validation
    let notification_result = match process_notification_request(
        event_type,
        request_params,
        &payload_value,
        OperationType::Notify,
    ) {
        Ok(result) => result,
        Err(response) => return response,
    };

    tracing::Span::current().record("topic", &notification_result.topic);
    tracing::Span::current().record(
        "spatial_enabled",
        notification_result.spatial_metadata.is_some(),
    );

    // Convert payload for backend storage
    let payload_string = convert_payload_to_string(&payload.payload);

    // Save to backend storage (handles spatial metadata automatically)
    if let Err(e) = save_to_backend(
        &notification_result,
        payload_string.as_deref(),
        notification_backend.get_ref().as_ref(),
    )
    .await
    {
        error!(
            error = %e,
            topic = %notification_result.topic,
            "Failed to save notification to backend"
        );
        return processing_error_response("Notification Storage", e);
    }

    // Build success response
    let response = NotificationResponse {
        status: "success".to_string(),
        request_id: request_id.to_string(),
        processed_at: chrono::Utc::now().to_rfc3339(),
    };

    // Enhanced logging with spatial information
    if let Some(spatial_metadata) = &notification_result.spatial_metadata {
        info!(
            topic = %notification_result.topic,
            event_type = %notification_result.event_type,
            param_count = notification_result.canonicalized_params.len(),
            payload_type = ?get_payload_type_name(&payload.payload),
            bounding_box = %spatial_metadata.bounding_box,
            "Notification with spatial data processed and saved successfully"
        );
    } else {
        info!(
            topic = %notification_result.topic,
            event_type = %notification_result.event_type,
            param_count = notification_result.canonicalized_params.len(),
            payload_type = ?get_payload_type_name(&payload.payload),
            "Notification processed and saved successfully"
        );
    }

    HttpResponse::Ok().json(response)
}
