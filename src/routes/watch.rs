use crate::error::validation_error_response;
use crate::handlers;
use crate::handlers::validate_operation_for_endpoint;
use crate::notification::OperationType;
use crate::notification_backend::NotificationBackend;
use actix_web::{HttpResponse, web};
use serde_json::Value;
use std::sync::Arc;
use tracing::info;
use tracing_actix_web::RequestId;

/// Watch endpoint handler that only supports watch and replay operations
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
pub async fn watch(
    payload: web::Json<Value>,
    notification_backend: web::Data<Arc<dyn NotificationBackend>>,
    request_id: RequestId,
) -> HttpResponse {
    // Process CloudEvent (includes operation extraction and validation)
    let cloudevent_response = match handlers::cloudevent::process_cloudevent(&payload).await {
        Ok(response) => response,
        Err(e) => return validation_error_response("CloudEvent", e),
    };

    // Validate that only watch and replay operations are allowed on this endpoint
    if let Err(e) = validate_operation_for_endpoint(
        cloudevent_response.operation,
        &[OperationType::Watch, OperationType::Replay],
        "watch",
    ) {
        return validation_error_response("Unsupported Operation", e);
    }

    // Process Aviso notification with the already validated operation
    let notification_result =
        match handlers::notification::process_aviso_request(&payload, &cloudevent_response).await {
            Ok(result) => result,
            Err(e) => return validation_error_response("Aviso Notification", e),
        };

    let operation_str = format!("{:?}", cloudevent_response.operation).to_lowercase();
    tracing::Span::current().record("operation", &operation_str);

    // For now, we just validate and return ok
    // Future implementation will set up actual watch subscriptions or replay queries
    info!(
        topic = %notification_result.topic,
        event_type = %notification_result.event_type,
        operation = %operation_str,
        "Watch request processed successfully (validation only)"
    );

    HttpResponse::Ok().json(serde_json::json!({
        "status": "ok"
    }))
}
