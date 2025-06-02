//! Core notification processing logic
//!
//! The processor handles the validation, canonicalization, and topic building
//! for notification requests. It supports both schema-driven processing for
//! known event types and generic fallback processing for unknown types.

use anyhow::{Context, Result, bail};
use std::collections::HashMap;

use crate::configuration::{EventSchema, ValidationRules};
use crate::notification::topic_builder::TopicBuilder;
use crate::notification::validators::*;
use crate::notification::{NotificationRegistry, OperationType, ProcessingResult};

/// Main processor for notification validation and canonicalization
///
/// The processor orchestrates the complete validation pipeline:
/// - Determines if schema-based or generic processing should be used
/// - Validates and canonicalizes all request parameters
/// - Builds appropriate topic strings for backend routing
/// - Extracts payload data if configured
pub struct NotificationProcessor<'a> {
    /// Reference to the schema registry for rule lookup
    registry: &'a NotificationRegistry,
}

impl<'a> NotificationProcessor<'a> {
    /// Create a new processor with access to the schema registry
    ///
    /// # Arguments
    /// * `registry` - Reference to the schema registry
    pub fn new(registry: &'a NotificationRegistry) -> Self {
        Self { registry }
    }

    /// Process request parameters based on event type and operation mode
    ///
    /// This is the main entry point for processing. It determines whether to use
    /// schema-based validation or generic processing based on whether a schema
    /// exists for the given event type.
    ///
    /// # Arguments
    /// * `event_type` - The type of event being processed
    /// * `request_params` - The request parameters to validate
    /// * `operation` - Whether this is a notify or watch operation
    ///
    /// # Returns
    /// * `Ok(ProcessingResult)` - Successfully processed with topic and payload
    /// * `Err(anyhow::Error)` - Validation or processing failed
    pub fn process_request(
        &self,
        event_type: &str,
        request_params: &HashMap<String, String>,
        operation: OperationType,
    ) -> Result<ProcessingResult> {
        // Determine processing strategy based on schema availability
        let canonicalized_params = if self.registry.has_schema(event_type) {
            // Use schema-based validation for known event types
            let schema = self.registry.get_schema(event_type).unwrap();
            match operation {
                OperationType::Notify => self.process_notify_request(schema, request_params)?,
                OperationType::Watch => self.process_watch_request(schema, request_params)?,
                OperationType::Replay => self.process_replay_request(schema, request_params)?,
            }
        } else {
            // Use generic validation for unknown event types
            self.process_generic_request(request_params, operation)?
        };

        // Build topic string based on schema or generic rules
        let topic = if let Some(schema) = self.registry.get_schema(event_type) {
            TopicBuilder::build_topic_with_schema(event_type, schema, &canonicalized_params)?
        } else {
            TopicBuilder::build_generic_topic(event_type, &canonicalized_params)
        };

        // Extract payload if schema defines payload configuration
        let payload = if let Some(schema) = self.registry.get_schema(event_type) {
            self.extract_payload(schema, request_params)?
        } else {
            None
        };

        Ok(ProcessingResult {
            event_type: event_type.to_string(),
            topic,
            payload,
            canonicalized_params,
        })
    }

    /// Process request for notify operation with schema validation
    ///
    /// For notify operations, ALL fields defined in the schema must be present
    /// and valid. This ensures complete data for storage in the backend.
    ///
    /// # Arguments
    /// * `schema` - The schema definition for this event type
    /// * `request_params` - The request parameters to validate
    ///
    /// # Returns
    /// * `Ok(HashMap)` - All fields validated and canonicalized
    /// * `Err(anyhow::Error)` - Missing required field or validation failed
    fn process_notify_request(
        &self,
        schema: &EventSchema,
        request_params: &HashMap<String, String>,
    ) -> Result<HashMap<String, String>> {
        let mut canonicalized = HashMap::new();

        // For notify operations, ALL schema fields must be present and valid
        for (field_name, rules) in &schema.request {
            let value = request_params.get(field_name).context(format!(
                "Required field '{}' missing for notify operation",
                field_name
            ))?;

            let canonicalized_value =
                self.validate_and_canonicalize_field(field_name, value, rules)?;

            canonicalized.insert(field_name.clone(), canonicalized_value);
        }

        Ok(canonicalized)
    }

    /// Process request for watch operation with schema validation
    ///
    /// For watch operations, only fields marked as required must be present.
    /// Missing optional fields are filled with "*" for wildcard matching.
    ///
    /// # Arguments
    /// * `schema` - The schema definition for this event type
    /// * `request_params` - The request parameters to validate
    ///
    /// # Returns
    /// * `Ok(HashMap)` - Required fields validated, optional fields as "*"
    /// * `Err(anyhow::Error)` - Missing required field or validation failed
    fn process_watch_request(
        &self,
        schema: &EventSchema,
        request_params: &HashMap<String, String>,
    ) -> Result<HashMap<String, String>> {
        let mut canonicalized = HashMap::new();

        for (field_name, rules) in &schema.request {
            let is_required = rules.iter().any(|rule| rule.is_required());

            if let Some(value) = request_params.get(field_name) {
                // Field is provided, validate and canonicalize it
                let canonicalized_value =
                    self.validate_and_canonicalize_field(field_name, value, rules)?;
                canonicalized.insert(field_name.clone(), canonicalized_value);
            } else if is_required {
                // Required field is missing - this is an error
                bail!(
                    "Required field '{}' missing for watch operation",
                    field_name
                );
            } else {
                // Optional field not provided, use "*" for wildcard matching
                canonicalized.insert(field_name.clone(), "*".to_string());
            }
        }

        Ok(canonicalized)
    }

    /// Process request for replay operation with schema validation
    ///
    /// For replay operations, only fields marked as required must be present.
    /// Missing optional fields are filled with "*" for wildcard matching.
    /// This is similar to watch operations but intended for historical data retrieval.
    fn process_replay_request(
        &self,
        schema: &EventSchema,
        request_params: &HashMap<String, String>,
    ) -> Result<HashMap<String, String>> {
        let mut canonicalized = HashMap::new();

        for (field_name, rules) in &schema.request {
            let is_required = rules.iter().any(|rule| rule.is_required());

            if let Some(value) = request_params.get(field_name) {
                // Field is provided, validate and canonicalize it
                let canonicalized_value =
                    self.validate_and_canonicalize_field(field_name, value, rules)?;
                canonicalized.insert(field_name.clone(), canonicalized_value);
            } else if is_required {
                // Required field is missing - this is an error
                bail!(
                    "Required field '{}' missing for replay operation",
                    field_name
                );
            } else {
                // Optional field not provided, use "*" for wildcard matching
                canonicalized.insert(field_name.clone(), "*".to_string());
            }
        }

        Ok(canonicalized)
    }

    /// Process request without schema using generic validation
    ///
    /// Generic processing provides basic validation without schema rules:
    /// - Notify: All provided values must be non-empty
    /// - Listen: All provided values are accepted as-is
    ///
    /// # Arguments
    /// * `request_params` - The request parameters to validate
    /// * `operation` - Whether this is a notify or watch operation
    ///
    /// # Returns
    /// * `Ok(HashMap)` - Parameters validated according to generic rules
    /// * `Err(anyhow::Error)` - Generic validation failed
    fn process_generic_request(
        &self,
        request_params: &HashMap<String, String>,
        operation: OperationType,
    ) -> Result<HashMap<String, String>> {
        let mut canonicalized = HashMap::new();

        match operation {
            OperationType::Notify => {
                // For notify without schema, ensure all values are non-empty
                for (key, value) in request_params {
                    if value.is_empty() {
                        bail!("Field '{}' cannot be empty", key);
                    }
                    canonicalized.insert(key.clone(), value.clone());
                }
            }
            OperationType::Watch | OperationType::Replay => {
                // For watch/replay without schema, accept any values including empty
                for (key, value) in request_params {
                    canonicalized.insert(key.clone(), value.clone());
                }
            }
        }

        Ok(canonicalized)
    }

    /// Validate and canonicalize a single field using its schema rules
    ///
    /// Applies the first validation rule found for the field. Multiple rules
    /// per field are supported in the schema but currently only the first
    /// rule is applied.
    ///
    /// # Arguments
    /// * `field_name` - Name of the field being validated
    /// * `value` - The value to validate
    /// * `rules` - List of validation rules for this field
    ///
    /// # Returns
    /// * `Ok(String)` - Validated and canonicalized value
    /// * `Err(anyhow::Error)` - Validation failed with detailed error
    fn validate_and_canonicalize_field(
        &self,
        field_name: &str,
        value: &str,
        rules: &[ValidationRules],
    ) -> Result<String> {
        // Apply the first rule (schema design assumes one rule per field)
        let rule = rules.first().context(format!(
            "No validation rules found for field '{}'",
            field_name
        ))?;

        match rule {
            ValidationRules::StringHandler { max_length, .. } => {
                StringHandler::validate_and_canonicalize(value, *max_length, field_name)
            }
            ValidationRules::DateHandler {
                canonical_format, ..
            } => DateHandler::validate_and_canonicalize(value, canonical_format, field_name),
            ValidationRules::EnumHandler { values, .. } => {
                EnumHandler::validate_and_canonicalize(value, values, field_name)
            }
            ValidationRules::ExpverHandler { default, .. } => {
                ExpverHandler::validate_and_canonicalize(value, default.as_deref(), field_name)
            }
            ValidationRules::IntHandler { range, .. } => {
                IntHandler::validate_and_canonicalize(value, range.as_ref(), field_name)
            }
            ValidationRules::TimeHandler { .. } => {
                TimeHandler::validate_and_canonicalize(value, field_name)
            }
        }
    }

    /// Extract payload data based on schema configuration
    ///
    /// If the schema defines a payload configuration, this method extracts
    /// the specified field value from the request parameters.
    ///
    /// # Arguments
    /// * `schema` - The schema definition
    /// * `request_params` - The request parameters
    ///
    /// # Returns
    /// * `Ok(Some(String))` - Payload extracted successfully
    /// * `Ok(None)` - No payload configured or not required
    /// * `Err(anyhow::Error)` - Required payload missing
    fn extract_payload(
        &self,
        schema: &EventSchema,
        request_params: &HashMap<String, String>,
    ) -> Result<Option<String>> {
        if let Some(payload_config) = &schema.payload {
            if let Some(payload_value) = request_params.get(&payload_config.key) {
                Ok(Some(payload_value.clone()))
            } else if payload_config.required {
                bail!("Required payload field '{}' is missing", payload_config.key);
            } else {
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::{EventSchema, PayloadConfig, TopicConfig, ValidationRules};
    use std::collections::HashMap;

    fn create_test_schema() -> EventSchema {
        let mut request = HashMap::new();
        request.insert(
            "class".to_string(),
            vec![ValidationRules::StringHandler {
                max_length: Some(2),
                required: true,
            }],
        );
        request.insert(
            "destination".to_string(),
            vec![ValidationRules::StringHandler {
                max_length: None,
                required: true,
            }],
        );
        request.insert(
            "optional_field".to_string(),
            vec![ValidationRules::StringHandler {
                max_length: None,
                required: false,
            }],
        );

        EventSchema {
            payload: Some(PayloadConfig {
                key: "location".to_string(),
                required: true,
            }),
            topic: Some(TopicConfig {
                base: "test".to_string(),
                separator: ".".to_string(),
                key_order: vec!["class".to_string(), "destination".to_string()],
            }),
            endpoint: None,
            request,
        }
    }

    #[test]
    fn test_notify_request_missing_required_field() {
        let mut schemas = HashMap::new();
        schemas.insert("test_event".to_string(), create_test_schema());
        let registry = NotificationRegistry::from_config(&schemas);
        let processor = NotificationProcessor::new(&registry);

        let mut params = HashMap::new();
        params.insert("destination".to_string(), "SCL".to_string());
        // Missing required "class" field

        // Use process_request with a known event type
        let result = processor.process_request("test_event", &params, OperationType::Notify);
        assert!(result.is_err());
    }

    #[test]
    fn test_listen_request_with_wildcards() {
        let mut schemas = HashMap::new();
        schemas.insert("test_event".to_string(), create_test_schema());
        let registry = NotificationRegistry::from_config(&schemas);
        let processor = NotificationProcessor::new(&registry);

        let mut params = HashMap::new();
        params.insert("class".to_string(), "od".to_string());
        params.insert("destination".to_string(), "SCL".to_string());
        params.insert("location".to_string(), "/path/to/file".to_string());
        // Missing optional_field should get "*"

        let result = processor.process_request("test_event", &params, OperationType::Watch);
        assert!(result.is_ok());

        let processing_result = result.unwrap();
        assert_eq!(
            processing_result.canonicalized_params.get("optional_field"),
            Some(&"*".to_string())
        );
    }

    #[test]
    fn test_payload_extraction_required_missing() {
        let mut schemas = HashMap::new();
        schemas.insert("test_event".to_string(), create_test_schema());
        let registry = NotificationRegistry::from_config(&schemas);
        let processor = NotificationProcessor::new(&registry);

        let mut params = HashMap::new();
        params.insert("class".to_string(), "od".to_string());
        params.insert("destination".to_string(), "SCL".to_string());
        // Missing required "location" payload field

        let result = processor.process_request("test_event", &params, OperationType::Notify);
        assert!(result.is_err());
    }

    #[test]
    fn test_payload_extraction_optional_missing() {
        // Create a schema with optional payload configuration
        let mut request = HashMap::new();
        request.insert(
            "class".to_string(),
            vec![ValidationRules::StringHandler {
                max_length: Some(2),
                required: true,
            }],
        );
        request.insert(
            "destination".to_string(),
            vec![ValidationRules::StringHandler {
                max_length: None,
                required: true,
            }],
        );

        let schema = EventSchema {
            payload: Some(PayloadConfig {
                key: "optional_payload".to_string(),
                required: false, // This is the key - payload is optional
            }),
            topic: Some(TopicConfig {
                base: "test".to_string(),
                separator: ".".to_string(),
                key_order: vec!["class".to_string(), "destination".to_string()],
            }),
            endpoint: None,
            request,
        };

        let mut schemas = HashMap::new();
        schemas.insert("test_event".to_string(), schema);
        let registry = NotificationRegistry::from_config(&schemas);
        let processor = NotificationProcessor::new(&registry);

        let mut params = HashMap::new();
        params.insert("class".to_string(), "od".to_string());
        params.insert("destination".to_string(), "SCL".to_string());
        // Missing optional payload field "optional_payload"

        let result = processor.process_request("test_event", &params, OperationType::Notify);
        assert!(result.is_ok());

        let processing_result = result.unwrap();
        assert!(processing_result.payload.is_none());
    }

    #[test]
    fn test_generic_processing_empty_values() {
        let registry = NotificationRegistry::new();
        let processor = NotificationProcessor::new(&registry);

        let mut params = HashMap::new();
        params.insert("field1".to_string(), "".to_string()); // Empty value
        params.insert("field2".to_string(), "valid".to_string());

        let result = processor.process_generic_request(&params, OperationType::Notify);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("cannot be empty"));
    }

    #[test]
    fn test_validation_rule_not_found() {
        let mut request = HashMap::new();
        request.insert("field".to_string(), vec![]); // No validation rules

        let schema = EventSchema {
            payload: None,
            topic: None,
            endpoint: None,
            request,
        };

        let mut schemas = HashMap::new();
        schemas.insert("test_event".to_string(), schema);
        let registry = NotificationRegistry::from_config(&schemas);
        let processor = NotificationProcessor::new(&registry);

        let mut params = HashMap::new();
        params.insert("field".to_string(), "value".to_string());

        let result = processor.process_request("test_event", &params, OperationType::Notify);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("No validation rules found")
        );
    }
}
