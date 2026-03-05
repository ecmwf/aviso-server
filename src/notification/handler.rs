//! High-level notification entry points.

use actix_web::web;
use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;
use tracing::debug;

use super::{NotificationProcessor, NotificationRegistry, OperationType, ProcessingResult};
use crate::configuration::Settings;
use crate::telemetry::{SERVICE_NAME, SERVICE_VERSION};

/// Public facade over registry + processor.
pub struct NotificationHandler {
    /// Schema registry used by the processor.
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

    /// Validate request parameters and build routing topic.
    pub fn process_request(
        &self,
        event_type: &str,
        request_params: &HashMap<String, String>,
        payload: &Option<serde_json::Value>,
        operation: OperationType,
    ) -> Result<ProcessingResult> {
        let processor = NotificationProcessor::new(&self.registry);
        processor.process_request(event_type, request_params, payload, operation)
    }

    /// Get all identifier keys defined for an event type.
    pub fn get_identifier_keys(&self, event_type: &str) -> Result<Vec<String>> {
        self.registry.get_identifier_keys(event_type)
    }

    /// Get required identifier keys for an event type.
    pub fn get_required_identifier_keys(&self, event_type: &str) -> Result<Vec<String>> {
        self.registry.get_required_identifier_keys(event_type)
    }

    /// Get full schema map.
    pub fn get_whole_schema(&self) -> &HashMap<String, crate::configuration::EventSchema> {
        self.registry.get_whole_schema()
    }
}

/// Extract and process Aviso notification from a CloudEvent payload.
pub fn extract_aviso_notification(
    payload: &web::Json<Value>,
    operation: OperationType,
) -> Result<ProcessingResult> {
    let (event_type, request_params) = extract_aviso_data(payload)?;

    let notification_handler =
        NotificationHandler::from_config(Settings::get_global_notification_schema().as_ref());

    // Payload is already carried by CloudEvent data; request validation path uses `None`.
    let mut processing_result =
        notification_handler.process_request(&event_type, &request_params, &None, operation)?;

    processing_result.event_type = event_type;

    tracing::info!(
        service_name = SERVICE_NAME,
        service_version = SERVICE_VERSION,
        event_domain = "notification",
        event_name = "notification.aviso.processed",
        operation = ?operation,
        aviso_event_type = processing_result.event_type,
        "Aviso notification processed with operation"
    );

    Ok(processing_result)
}

/// Extract Aviso event type + identifier fields from CloudEvent data.
fn extract_aviso_data(payload: &web::Json<Value>) -> Result<(String, HashMap<String, String>)> {
    let data = payload
        .get("data")
        .ok_or_else(|| anyhow::anyhow!("Aviso CloudEvents must include a 'data' field"))?;

    let event_type = data
        .get("event")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required 'event' field in Aviso data"))?
        .to_string();

    let identifier_obj = data
        .get("identifier")
        .and_then(|v| v.as_object())
        .ok_or_else(|| anyhow::anyhow!("Missing required 'identifier' field in Aviso data"))?;

    // Identifier values are stringly typed in downstream schema validation.
    let mut request_params = HashMap::new();
    for (key, value) in identifier_obj {
        let string_value = match value.as_str() {
            Some(s) => s.to_string(),
            None => {
                debug!(key = key, "Converting non-string value to string");
                value.to_string().trim_matches('"').to_string()
            }
        };
        request_params.insert(key.clone(), string_value);
    }

    // Preserve optional Aviso fields that may exist outside `identifier`.
    extract_additional_fields(data, &mut request_params);

    debug!(
        event_type = %event_type,
        param_count = request_params.len(),
        "Extracted Aviso request parameters"
    );

    Ok((event_type, request_params))
}

/// Copy optional Aviso fields used by downstream processing.
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
                "identifier": {
                    "class": "od"
                }
                // Missing "event" field
            }
        }));

        let result = extract_aviso_data(&payload);
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_aviso_data_missing_identifier_field() {
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
                .contains("Missing required 'identifier' field")
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
                "identifier": {
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
                "identifier": {
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
