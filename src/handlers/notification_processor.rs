// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use crate::configuration::Settings;
use crate::notification::{NotificationHandler, OperationType, ProcessingResult};
use crate::telemetry::{SERVICE_NAME, SERVICE_VERSION};
use anyhow::Result;
use serde_json::Value;
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
/// - Schema-level payload requirement checks
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
    request_params: &HashMap<String, Value>,
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
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_name = "notification.processing.failed",
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
    error.chain().any(|cause| {
        let message = cause.to_string().to_ascii_lowercase();
        message.starts_with("required field ")
            || message.starts_with("payload is required")
            || message.contains("unknown event type")
            || message.starts_with("field '")
            || message.contains("must be a valid")
    })
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
