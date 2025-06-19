use crate::configuration::Settings;
use crate::error::validation_error_response;
use crate::notification::validators::validate_payload_type;
use crate::notification::{NotificationHandler, OperationType, ProcessingResult};
use crate::types::PayloadType;
use actix_web::HttpResponse;
use std::collections::HashMap;

/// Process notification request with validation
pub fn process_notification_request(
    event_type: &str,
    request_params: &HashMap<String, String>,
    payload: &Option<PayloadType>,
    operation_type: OperationType,
) -> Result<ProcessingResult, HttpResponse> {
    // Validate payload type against schema configuration
    if let Err(e) = validate_payload_type(event_type, payload) {
        return Err(validation_error_response("Payload Type", e));
    }

    // Process notification
    let notification_handler =
        NotificationHandler::from_config(Settings::get_global_notification_schema().as_ref());

    match notification_handler.process_request(event_type, request_params, operation_type) {
        Ok(result) => Ok(result),
        Err(e) => Err(validation_error_response("Notification", e)),
    }
}
