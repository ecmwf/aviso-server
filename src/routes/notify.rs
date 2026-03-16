use crate::error::{
    ProcessingKind, RequestKind, processing_error_response, request_parse_error_response,
    request_validation_error_response,
};
use crate::handlers::{
    NotificationErrorKind, parse_and_validate_request, process_notification_request,
    save_to_backend,
};
use crate::notification::OperationType;
use crate::notification::decode_subject_for_display;
use crate::notification_backend::NotificationBackend;
use crate::routes::streaming::enforce_stream_auth;
use crate::telemetry::{SERVICE_NAME, SERVICE_VERSION};
use crate::types::{NotificationRequest, NotificationResponse};
use actix_web::{HttpRequest, HttpResponse, web};
use std::sync::Arc;
use tracing::info;
use tracing_actix_web::RequestId;

/// Notification endpoint handler
///
/// Processes notification requests with all schema fields required.
/// Validates request format, processes notification, and saves to backend.
/// Now supports spatial metadata extraction for polygon fields.
#[utoipa::path(
    post,
    path = "/api/v1/notification",
    tag = "notification",
    request_body = NotificationRequest,
    responses(
        (status = 200, description = "Notification processed and stored successfully", body = crate::types::NotificationResponse),
        (status = 400, description = "Invalid request data or validation failure"),
        (status = 401, description = "Missing or invalid credentials (when stream requires auth)"),
        (status = 403, description = "Valid credentials but insufficient roles for this stream"),
        (status = 500, description = "Internal server error during processing"),
        (status = 503, description = "Authentication service unavailable (direct mode)")
    ),
    security(
        ("bearer_jwt" = []),
        ("basic" = []),
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
    http_request: HttpRequest,
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

    // Reject unauthorized requests before validation/topic work.
    if let Err(response) = enforce_stream_auth(&http_request, event_type) {
        return response;
    }

    // Process notification request with payload validation
    let notification_result = match process_notification_request(
        event_type,
        request_params,
        &payload.payload,
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

    // Payload is always persisted as canonical JSON.
    // Missing optional payload is represented as JSON null.
    let payload_string = payload
        .payload
        .as_ref()
        .map(serde_json::Value::to_string)
        .unwrap_or_else(|| "null".to_string());

    // Save to backend storage (handles spatial metadata automatically)
    if let Err(e) = save_to_backend(
        &notification_result,
        payload_string,
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

    // Emit one success event with optional spatial metadata instead of branching logs.
    let payload_kind = payload
        .payload
        .as_ref()
        .map(json_value_kind)
        .unwrap_or("null");
    let spatial_enabled = notification_result.spatial_metadata.is_some();
    let spatial_bbox = notification_result
        .spatial_metadata
        .as_ref()
        .map(|metadata| metadata.bounding_box.as_str());
    info!(
        service_name = SERVICE_NAME,
        service_version = SERVICE_VERSION,
        event_name = "api.notification.processed",
        outcome = "success",
        topic = %display_topic,
        event_type = %notification_result.event_type,
        param_count = notification_result.canonicalized_params.len(),
        payload_kind = %payload_kind,
        spatial_enabled = spatial_enabled,
        spatial_bbox = ?spatial_bbox,
        "Notification processed and saved successfully"
    );

    HttpResponse::Ok().json(response)
}

fn json_value_kind(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}
