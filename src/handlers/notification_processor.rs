use crate::configuration::Settings;
use crate::notification::{NotificationHandler, OperationType, ProcessingResult};
use anyhow::Result;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationErrorKind {
    Validation,
    Processing,
}

pub struct NotificationProcessingError {
    pub kind: NotificationErrorKind,
    pub source: anyhow::Error,
}

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
/// * `Err(NotificationProcessingError)` - Validation or processing failure
pub fn process_notification_request(
    event_type: &str,
    request_params: &HashMap<String, String>,
    payload: &Option<serde_json::Value>,
    operation: OperationType,
) -> Result<ProcessingResult, NotificationProcessingError> {
    let handler =
        NotificationHandler::from_config(Settings::get_global_notification_schema().as_ref());

    match handler.process_request(event_type, request_params, payload, operation) {
        Ok(result) => Ok(result),
        Err(e) => {
            let kind = if is_notification_validation_error(&e) {
                NotificationErrorKind::Validation
            } else {
                NotificationErrorKind::Processing
            };
            tracing::warn!(
                event_type = %event_type,
                error_kind = ?kind,
                error = %e,
                "Notification processing failed"
            );
            Err(NotificationProcessingError { kind, source: e })
        }
    }
}

fn is_notification_validation_error(error: &anyhow::Error) -> bool {
    let text = error.to_string().to_ascii_lowercase();
    text.contains("required field")
        || text.contains("unknown event type")
        || text.contains("must be")
        || text.contains("invalid")
        || text.contains("payload")
        || text.contains("cannot be empty")
}

#[cfg(test)]
mod tests {
    use super::is_notification_validation_error;

    #[test]
    fn classifies_validation_like_messages() {
        let err = anyhow::anyhow!("Required field 'class' missing for notify operation");
        assert!(is_notification_validation_error(&err));
    }

    #[test]
    fn classifies_non_validation_messages_as_processing() {
        let err = anyhow::anyhow!("failed to initialize notification registry");
        assert!(!is_notification_validation_error(&err));
    }
}
