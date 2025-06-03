use crate::cloudevents::{
    validation::AvisoTypeValidator, validation::extract_and_validate_aviso_operation,
};
use crate::error::validation_error_response;
use crate::handlers::{process_aviso, process_cloudevent};
use actix_web::{HttpResponse, web};
use serde_json::Value;
use tracing::error;

#[tracing::instrument(
    skip(payload),
    fields(
        event_id = tracing::field::Empty,
        event_type = tracing::field::Empty,
    )
)]
pub async fn notify(payload: web::Json<Value>) -> HttpResponse {
    // Process CloudEvent
    let cloudevent_response = match process_cloudevent(&payload).await {
        Ok(response) => response,
        Err(e) => return validation_error_response("CloudEvent", e),
    };

    // Validate this is an Aviso CloudEvent with dynamic error message
    if let Err(e) = AvisoTypeValidator::validate_is_aviso_type(&cloudevent_response.event_type) {
        error!(
            event_type = %cloudevent_response.event_type,
            "Rejected non-Aviso CloudEvent"
        );
        return validation_error_response("CloudEvent Type", e);
    }

    // Extract and validate Aviso operation type
    let operation = match extract_and_validate_aviso_operation(&cloudevent_response.event_type) {
        Ok(op) => op,
        Err(e) => return validation_error_response("Aviso CloudEvent Type", e),
    };

    // Process Aviso notification
    if let Err(e) = process_aviso(&payload, operation).await {
        return validation_error_response("Aviso Notification", e);
    }

    HttpResponse::Ok().json(cloudevent_response)
}
