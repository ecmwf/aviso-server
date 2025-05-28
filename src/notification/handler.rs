//! High-level notification processing and CloudEvent integration
//!
//! This module provides the main public API for notification processing,
//! including CloudEvent integration and the primary NotificationHandler.

use actix_web::web;
use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;
use tracing::debug;

use super::{NotificationProcessor, NotificationRegistry, OperationType, ProcessingResult};
use crate::configuration::Settings;

/// Main notification handler that orchestrates the entire processing pipeline
///
/// This is the primary entry point for notification processing. It coordinates
/// between the registry (schema management) and processor (validation/canonicalization)
/// to provide a clean API for the rest of the application.
pub struct NotificationHandler {
    /// Registry containing all configured schemas and validation rules
    registry: NotificationRegistry,
}

impl NotificationHandler {
    /// Create a new notification handler from configuration
    pub fn from_config(
        notification_schema: Option<&HashMap<String, crate::configuration::EventSchema>>,
    ) -> Self {
        let registry = if let Some(schemas) = notification_schema {
            NotificationRegistry::from_config(schemas)
        } else {
            NotificationRegistry::new()
        };

        Self { registry }
    }

    /// Process a notification request with validation and canonicalization
    pub fn process_request(
        &self,
        event_type: &str,
        request_params: &HashMap<String, String>,
        operation: OperationType,
    ) -> Result<ProcessingResult> {
        let processor = NotificationProcessor::new(&self.registry);
        processor.process_request(event_type, request_params, operation)
    }

    /// Get all request keys defined in the schema for a specific event type
    pub fn get_request_keys(&self, event_type: &str) -> Result<Vec<String>> {
        self.registry.get_request_keys(event_type)
    }

    /// Get only the required request keys for a specific event type
    pub fn get_required_request_keys(&self, event_type: &str) -> Result<Vec<String>> {
        self.registry.get_required_request_keys(event_type)
    }

    /// Get the complete schema configuration
    pub fn get_whole_schema(&self) -> &HashMap<String, crate::configuration::EventSchema> {
        self.registry.get_whole_schema()
    }
}

/// Extract and process Aviso notification from CloudEvent payload
///
/// This function handles the complete Aviso notification pipeline:
/// - Extracts and validates Aviso data structure
/// - Converts request parameters (assuming they're already strings)
/// - Applies schema-based validation and canonicalization
pub fn extract_aviso_notification(
    payload: &web::Json<Value>,
    operation: OperationType,
) -> Result<ProcessingResult> {
    // Extract Aviso data structure
    let (event_type, request_params) = extract_aviso_data(payload)?;

    // Process with notification handler using the provided operation
    let notification_handler =
        NotificationHandler::from_config(Settings::get_global_notification_schema().as_ref());

    let mut processing_result =
        notification_handler.process_request(&event_type, &request_params, operation)?;

    processing_result.event_type = event_type;

    tracing::info!(
        operation = ?operation,
        aviso_event_type = processing_result.event_type,
        "Aviso notification processed with operation"
    );

    Ok(processing_result)
}

/// Extract Aviso data from CloudEvent payload
fn extract_aviso_data(payload: &web::Json<Value>) -> Result<(String, HashMap<String, String>)> {
    let data = payload
        .get("data")
        .ok_or_else(|| anyhow::anyhow!("Aviso CloudEvents must include a 'data' field"))?;

    let event_type = data
        .get("event")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required 'event' field in Aviso data"))?
        .to_string();

    let request_obj = data
        .get("request")
        .and_then(|v| v.as_object())
        .ok_or_else(|| anyhow::anyhow!("Missing required 'request' field in Aviso data"))?;

    // Extract request parameters - most should already be strings
    let mut request_params = HashMap::new();
    for (key, value) in request_obj {
        let string_value = match value.as_str() {
            Some(s) => s.to_string(),
            None => {
                debug!(key = key, "Converting non-string value to string");
                value.to_string().trim_matches('"').to_string()
            }
        };
        request_params.insert(key.clone(), string_value);
    }

    // Extract additional Aviso fields
    extract_additional_fields(data, &mut request_params);

    debug!(
        event_type = %event_type,
        param_count = request_params.len(),
        "Extracted Aviso request parameters"
    );

    Ok((event_type, request_params))
}

/// Extract additional Aviso-specific fields like payload and location
fn extract_additional_fields(data: &Value, request_params: &mut HashMap<String, String>) {
    if let Some(payload_str) = data.get("payload").and_then(|v| v.as_str()) {
        request_params.insert("payload".to_string(), payload_str.to_string());
    }

    if let Some(location_str) = data.get("location").and_then(|v| v.as_str()) {
        request_params.insert("location".to_string(), location_str.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::web;
    use serde_json::json;

    #[test]
    fn test_extract_aviso_data_missing_data_field() {
        let payload = web::Json(json!({
            "specversion": "1.0",
            "id": "test",
            "source": "/test",
            "type": "int.ecmwf.aviso.notify"
            // Missing "data" field
        }));

        let result = extract_aviso_data(&payload);
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_aviso_data_missing_event_field() {
        let payload = web::Json(json!({
            "specversion": "1.0",
            "id": "test",
            "source": "/test",
            "type": "int.ecmwf.aviso.notify",
            "data": {
                "request": {
                    "class": "od"
                }
                // Missing "event" field
            }
        }));

        let result = extract_aviso_data(&payload);
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_aviso_data_missing_request_field() {
        let payload = web::Json(json!({
            "specversion": "1.0",
            "id": "test",
            "source": "/test",
            "type": "int.ecmwf.aviso.notify",
            "data": {
                "event": "dissemination"
                // Missing "request" field
            }
        }));

        let result = extract_aviso_data(&payload);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Missing required 'request' field")
        );
    }

    #[test]
    fn test_extract_additional_fields() {
        let payload = web::Json(json!({
            "specversion": "1.0",
            "id": "test",
            "source": "/test",
            "type": "int.ecmwf.aviso.notify",
            "data": {
                "event": "dissemination",
                "request": {
                    "class": "od"
                },
                "payload": "test-payload",
                "location": "/path/to/file"
            }
        }));

        let result = extract_aviso_data(&payload);
        assert!(result.is_ok());

        let (_, params) = result.unwrap();
        assert_eq!(params.get("payload"), Some(&"test-payload".to_string()));
        assert_eq!(params.get("location"), Some(&"/path/to/file".to_string()));
    }

    #[test]
    fn test_non_string_value_conversion() {
        let payload = web::Json(json!({
            "specversion": "1.0",
            "id": "test",
            "source": "/test",
            "type": "int.ecmwf.aviso.notify",
            "data": {
                "event": "dissemination",
                "request": {
                    "class": "od",
                    "step": 12, // Number value
                    "active": true // Boolean value
                }
            }
        }));

        let result = extract_aviso_data(&payload);
        assert!(result.is_ok());

        let (_, params) = result.unwrap();
        assert_eq!(params.get("step"), Some(&"12".to_string()));
        assert_eq!(params.get("active"), Some(&"true".to_string()));
    }
}
