use crate::cloudevents::AvisoTypeValidator;
use crate::cloudevents::handler::CloudEventResponse;
use crate::cloudevents::validation::extract_and_validate_aviso_operation;
use crate::notification::{self, ProcessingResult};
use actix_web::web;
use serde_json::Value;
use tracing::info;

#[derive(Debug, Clone, serde::Serialize)]
pub struct NotificationResponse {
    pub status: String,
    pub request_id: String,
    pub processed_at: String,
}

/// Process Aviso notification - pure business logic
///
/// This function extracts and processes Aviso notifications without any knowledge
/// of how or where the data will be stored. It focuses solely on the business logic
/// of building topics and extracting relevant data.
///
/// The function remains completely decoupled from infrastructure concerns.
pub async fn process_aviso_request(
    payload: &web::Json<Value>,
    cloudevent_response: &CloudEventResponse,
) -> Result<ProcessingResult, anyhow::Error> {
    // Validate Aviso type
    AvisoTypeValidator::validate_is_aviso_type(&cloudevent_response.event_type)?;

    // Extract operation type
    let operation = extract_and_validate_aviso_operation(&cloudevent_response.event_type)?;

    // Process Aviso notification
    let processing_result = notification::handler::extract_aviso_notification(payload, operation)?;

    // Update tracing context
    tracing::Span::current().record("topic", &processing_result.topic);

    info!(
        operation = ?operation,
        event_type = %processing_result.event_type,
        topic = %processing_result.topic,
        param_count = processing_result.canonicalized_params.len(),
        has_payload = processing_result.payload.is_some(),
        "Aviso notification processed successfully"
    );

    Ok(processing_result)
}
