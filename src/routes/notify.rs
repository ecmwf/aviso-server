use crate::error::{
    ProcessingKind, RequestKind, processing_error_response, request_parse_error_response,
    request_validation_error_response,
};
use crate::handlers::{
    NotificationErrorKind, convert_payload_to_string, get_payload_type_name,
    parse_and_validate_request, process_notification_request, save_to_backend,
};
use crate::notification::OperationType;
use crate::notification::decode_subject_for_display;
use crate::notification_backend::NotificationBackend;
use crate::telemetry::{SERVICE_NAME, SERVICE_VERSION};
use crate::types::{NotificationRequest, NotificationResponse};
use actix_web::{HttpResponse, web};
use std::sync::Arc;
use tracing::info;
use tracing_actix_web::RequestId;

/// Notification endpoint handler
///
/// Processes notification requests with all schema fields required.
/// Validates request format, payload type, processes notification, and saves to backend.
/// Now supports spatial metadata extraction for polygon fields.
#[utoipa::path(
    post,
    path = "/api/v1/notification",
    tag = "notification",
    request_body = NotificationRequest,
    responses(
        (status = 200, description = "Notification processed and stored successfully", body = crate::types::NotificationResponse),
        (status = 400, description = "Invalid request data or validation failure"),
        (status = 500, description = "Internal server error during processing")
    )
)]
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
        Err(e) => return request_parse_error_response(RequestKind::Notification, e),
    };
    if payload.identifier.contains_key("point") {
        return request_validation_error_response(
            RequestKind::Notification,
            anyhow::anyhow!(
                "identifier.point is only supported for watch/replay endpoints, not /notification"
            ),
        );
    }

    let event_type = &payload.event_type;
    let request_params = &payload.identifier;

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
        Err(e) => match e.kind {
            NotificationErrorKind::Validation => {
                return request_validation_error_response(RequestKind::Notification, e.source);
            }
            NotificationErrorKind::Processing => {
                return processing_error_response(ProcessingKind::NotificationProcessing, e.source);
            }
        },
    };

    let display_topic = decode_subject_for_display(&notification_result.topic);
    tracing::Span::current().record("topic", &display_topic);
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
        return processing_error_response(ProcessingKind::NotificationStorage, e);
    }

    // Build success response
    let response = NotificationResponse {
        status: "success".to_string(),
        request_id: request_id.to_string(),
        processed_at: chrono::Utc::now().to_rfc3339(),
    };

    // Enhanced logging with spatial information
    let payload_type = get_payload_type_name(&payload.payload).unwrap_or("None");
    if let Some(spatial_metadata) = &notification_result.spatial_metadata {
        info!(
            service_name = SERVICE_NAME,
            service_version = SERVICE_VERSION,
            event_domain = "notification",
            event_name = "api.notification.processed",
            outcome = "success",
            topic = %display_topic,
            event_type = %notification_result.event_type,
            param_count = notification_result.canonicalized_params.len(),
            payload_type = %payload_type,
            bounding_box = %spatial_metadata.bounding_box,
            "Notification with spatial data processed and saved successfully"
        );
    } else {
        info!(
            service_name = SERVICE_NAME,
            service_version = SERVICE_VERSION,
            event_domain = "notification",
            event_name = "api.notification.processed",
            outcome = "success",
            topic = %display_topic,
            event_type = %notification_result.event_type,
            param_count = notification_result.canonicalized_params.len(),
            payload_type = %payload_type,
            "Notification processed and saved successfully"
        );
    }

    HttpResponse::Ok().json(response)
}
