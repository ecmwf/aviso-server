use crate::error::{processing_error_response, validation_error_response};
use crate::handlers;
use crate::handlers::save_to_backend;
use crate::handlers::validate_operation_for_endpoint;
use crate::notification::OperationType;
use crate::notification_backend::NotificationBackend;
use actix_web::{HttpResponse, web};
use serde_json::Value;
use std::sync::Arc;
use tracing::{error, info};
use tracing_actix_web::RequestId;

// Proper response structure for notify endpoint
#[derive(Debug, Clone, serde::Serialize)]
pub struct NotificationResponse {
    pub status: String,
    pub request_id: String,
    pub processed_at: String,
}

/// Simple notification endpoint handler that only supports notify operations
#[tracing::instrument(
    skip(payload, notification_backend),
    fields(
        event_id = tracing::field::Empty,
        event_type = tracing::field::Empty,
        topic = tracing::field::Empty,
        request_id = %request_id,
        operation = tracing::field::Empty,
    )
)]
pub async fn notify(
    payload: web::Json<Value>,
    notification_backend: web::Data<Arc<dyn NotificationBackend>>,
    request_id: RequestId,
) -> HttpResponse {
    // Process CloudEvent (includes operation extraction and validation)
    let cloudevent_response = match handlers::cloudevent::process_cloudevent(&payload).await {
        Ok(response) => response,
        Err(e) => return validation_error_response("CloudEvent", e),
    };

    // Validate that only notify operations are allowed on this endpoint
    if let Err(e) = validate_operation_for_endpoint(
        cloudevent_response.operation,
        &[OperationType::Notify],
        "notification",
    ) {
        return validation_error_response("Unsupported Operation", e);
    }

    // Process Aviso notification
    let notification_result =
        match handlers::notification::process_aviso_request(&payload, &cloudevent_response).await {
            Ok(result) => result,
            Err(e) => return validation_error_response("Aviso Notification", e),
        };

    // Save to backend
    if let Err(e) = save_to_backend(&notification_result, notification_backend.get_ref()).await {
        error!(
            error = %e,
            topic = %notification_result.topic,
            "Failed to save notification to backend"
        );
        return processing_error_response("Notification Storage", e);
    }

    let operation_str = format!("{:?}", cloudevent_response.operation).to_lowercase();
    tracing::Span::current().record("operation", &operation_str);

    info!(
        topic = %notification_result.topic,
        event_type = %notification_result.event_type,
        operation = %operation_str,
        "Notification processed and saved successfully"
    );

    // Return the proper NotificationResponse structure
    let response = NotificationResponse {
        status: "success".to_string(),
        request_id: request_id.to_string(),
        processed_at: chrono::Utc::now().to_rfc3339(),
    };

    HttpResponse::Ok().json(response)
}
