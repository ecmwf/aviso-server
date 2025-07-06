use crate::configuration::Settings;
use crate::error::processing_error_response;
use crate::notification::{NotificationHandler, OperationType, ProcessingResult};
use actix_web::HttpResponse;
use anyhow::Result;
use std::collections::HashMap;

/// Process notification request with schema validation and spatial metadata extraction
///
/// This function handles the core notification processing logic including:
/// - Schema-based validation of request parameters
/// - Spatial metadata extraction for polygon fields
/// - Payload type validation for spatial notifications
///
/// # Arguments
/// * `event_type` - The type of event being processed
/// * `request_params` - Request parameters to validate and canonicalize
/// * `payload` - Optional payload data as serde_json::Value
/// * `operation` - Type of operation (Notify, Watch, Replay)
///
/// # Returns
/// * `Ok(ProcessingResult)` - Successfully processed notification with spatial metadata
/// * `Err(HttpResponse)` - Processing error response
pub fn process_notification_request(
    event_type: &str,
    request_params: &HashMap<String, String>,
    payload: &Option<serde_json::Value>,
    operation: OperationType,
) -> Result<ProcessingResult, HttpResponse> {
    let handler =
        NotificationHandler::from_config(Settings::get_global_notification_schema().as_ref());

    match handler.process_request(event_type, request_params, payload, operation) {
        Ok(result) => Ok(result),
        Err(e) => {
            tracing::warn!(
                event_type = %event_type,
                error = %e,
                "Notification processing failed"
            );
            Err(processing_error_response("Notification Processing", e))
        }
    }
}
