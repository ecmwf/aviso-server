//! Core notification validation and topic construction.

use anyhow::{Context, Result, bail};
use std::collections::HashMap;

use crate::configuration::EventSchema;
use crate::notification::spatial::SpatialMetadata;
use crate::notification::topic_builder::TopicBuilder;
use crate::notification::{NotificationRegistry, OperationType, ProcessingResult};
use aviso_validators::ValidationRules;
use aviso_validators::{
    DateHandler, EnumHandler, ExpverHandler, IntHandler, PolygonHandler, StringHandler, TimeHandler,
};

/// Notification request processor.
pub struct NotificationProcessor<'a> {
    /// Schema registry for event lookups.
    registry: &'a NotificationRegistry,
}

impl<'a> NotificationProcessor<'a> {
    /// Create processor with schema registry.
    pub fn new(registry: &'a NotificationRegistry) -> Self {
        Self { registry }
    }

    /// Validate request fields and build topic for the selected operation.
    pub fn process_request(
        &self,
        event_type: &str,
        request_params: &HashMap<String, String>,
        payload: &Option<serde_json::Value>,
        operation: OperationType,
    ) -> Result<ProcessingResult> {
        // Schema-driven when available; generic fallback otherwise.
        let (canonicalized_params, spatial_metadata) = if self.registry.has_schema(event_type) {
            let schema = self.registry.get_schema(event_type).unwrap();
            match operation {
                OperationType::Notify => {
                    self.process_notify_request(schema, request_params, payload)?
                }
                OperationType::Watch => (self.process_watch_request(schema, request_params)?, None),
                OperationType::Replay => {
                    (self.process_replay_request(schema, request_params)?, None)
                }
            }
        } else {
            (
                self.process_generic_request(request_params, operation)?,
                None,
            )
        };

        // Topic always comes from canonicalized values.
        let topic = if let Some(schema) = self.registry.get_schema(event_type) {
            TopicBuilder::build_topic_with_schema(event_type, schema, &canonicalized_params)?
        } else {
            TopicBuilder::build_generic_topic(event_type, &canonicalized_params)
        };

        Ok(ProcessingResult {
            event_type: event_type.to_string(),
            topic,
            canonicalized_params,
            spatial_metadata,
        })
    }

    /// Notify mode: every schema field must be present and valid.
    fn process_notify_request(
        &self,
        schema: &EventSchema,
        request_params: &HashMap<String, String>,
        payload: &Option<serde_json::Value>,
    ) -> Result<(HashMap<String, String>, Option<SpatialMetadata>)> {
        let mut canonicalized = HashMap::new();
        let mut spatial_metadata = None;
        let mut has_polygon_field = false;

        for (field_name, rules) in &schema.identifier {
            let value = request_params.get(field_name).context(format!(
                "Required field '{}' missing for notify operation",
                field_name
            ))?;

            let canonicalized_value =
                self.validate_and_canonicalize_field(field_name, value, rules)?;

            // Polygon fields attach spatial metadata for downstream filtering.
            if matches!(rules.first(), Some(ValidationRules::PolygonHandler { .. })) {
                has_polygon_field = true;
                let coordinates = PolygonHandler::parse_polygon_coordinates(value)?;

                spatial_metadata = Some(SpatialMetadata::from_coordinates(&coordinates)?);

                tracing::debug!(
                    field_name = field_name,
                    coordinate_count = coordinates.len(),
                    bounding_box = %spatial_metadata.as_ref().unwrap().bounding_box,
                    "Extracted spatial metadata from polygon field"
                );
            }

            canonicalized.insert(field_name.clone(), canonicalized_value);
        }

        // Polygon notifications require object payload for spatial data.
        if has_polygon_field {
            self.validate_polygon_payload_type(payload)?;
        }

        Ok((canonicalized, spatial_metadata))
    }

    /// Polygon payload must be a JSON object.
    fn validate_polygon_payload_type(&self, payload: &Option<serde_json::Value>) -> Result<()> {
        match payload {
            Some(serde_json::Value::Object(_)) => Ok(()),
            Some(_) => {
                anyhow::bail!(
                    "When polygon field is specified, payload must be a HashMap/Object, not a primitive type"
                )
            }
            None => {
                anyhow::bail!(
                    "When polygon field is specified, payload is required and must be a HashMap/Object"
                )
            }
        }
    }

    /// Watch mode: required fields are enforced, optional fields become `"*"`.
    fn process_watch_request(
        &self,
        schema: &EventSchema,
        request_params: &HashMap<String, String>,
    ) -> Result<HashMap<String, String>> {
        let mut canonicalized = HashMap::new();
        let has_point = request_params.contains_key("point");

        for (field_name, rules) in &schema.identifier {
            let is_required = rules.iter().any(|rule| rule.is_required());

            if let Some(value) = request_params.get(field_name) {
                let canonicalized_value =
                    self.validate_and_canonicalize_field(field_name, value, rules)?;
                canonicalized.insert(field_name.clone(), canonicalized_value);
            } else if is_required {
                if field_name == "polygon" && has_point {
                    canonicalized.insert(field_name.clone(), "*".to_string());
                    continue;
                }
                bail!(
                    "Required field '{}' missing for watch operation",
                    field_name
                );
            } else {
                // `"*"` keeps positional matching while representing "any value".
                canonicalized.insert(field_name.clone(), "*".to_string());
            }
        }

        if let Some(point) = request_params.get("point") {
            canonicalized.insert("point".to_string(), point.clone());
        }

        Ok(canonicalized)
    }

    /// Replay mode: same schema field rules as watch mode.
    fn process_replay_request(
        &self,
        schema: &EventSchema,
        request_params: &HashMap<String, String>,
    ) -> Result<HashMap<String, String>> {
        let mut canonicalized = HashMap::new();
        let has_point = request_params.contains_key("point");

        for (field_name, rules) in &schema.identifier {
            let is_required = rules.iter().any(|rule| rule.is_required());

            if let Some(value) = request_params.get(field_name) {
                let canonicalized_value =
                    self.validate_and_canonicalize_field(field_name, value, rules)?;
                canonicalized.insert(field_name.clone(), canonicalized_value);
            } else if is_required {
                if field_name == "polygon" && has_point {
                    canonicalized.insert(field_name.clone(), "*".to_string());
                    continue;
                }
                bail!(
                    "Required field '{}' missing for replay operation",
                    field_name
                );
            } else {
                canonicalized.insert(field_name.clone(), "*".to_string());
            }
        }

        if let Some(point) = request_params.get("point") {
            canonicalized.insert("point".to_string(), point.clone());
        }

        Ok(canonicalized)
    }

    /// Fallback validation for event types without schema.
    fn process_generic_request(
        &self,
        request_params: &HashMap<String, String>,
        operation: OperationType,
    ) -> Result<HashMap<String, String>> {
        let mut canonicalized = HashMap::new();

        match operation {
            OperationType::Notify => {
                for (key, value) in request_params {
                    if value.is_empty() {
                        bail!("Field '{}' cannot be empty", key);
                    }
                    canonicalized.insert(key.clone(), value.clone());
                }
            }
            OperationType::Watch | OperationType::Replay => {
                for (key, value) in request_params {
                    canonicalized.insert(key.clone(), value.clone());
                }
            }
        }

        Ok(canonicalized)
    }

    /// Validate one field with its configured rule.
    fn validate_and_canonicalize_field(
        &self,
        field_name: &str,
        value: &str,
        rules: &[ValidationRules],
    ) -> Result<String> {
        // Current schema model uses one effective rule per field.
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
            ValidationRules::PolygonHandler { .. } => {
                PolygonHandler::validate_and_canonicalize(value, field_name)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::{EventSchema, PayloadConfig, TopicConfig};
    use aviso_validators::ValidationRules;
    use std::collections::HashMap;

    fn create_test_schema() -> EventSchema {
        let mut identifier = HashMap::new();
        identifier.insert(
            "class".to_string(),
            vec![ValidationRules::StringHandler {
                max_length: Some(2),
                required: true,
            }],
        );
        identifier.insert(
            "destination".to_string(),
            vec![ValidationRules::StringHandler {
                max_length: None,
                required: true,
            }],
        );
        identifier.insert(
            "optional_field".to_string(),
            vec![ValidationRules::StringHandler {
                max_length: None,
                required: false,
            }],
        );

        EventSchema {
            payload: Some(PayloadConfig {
                allowed_types: vec!["String".to_string()],
                required: true,
            }),
            topic: Some(TopicConfig {
                base: "test".to_string(),
                key_order: vec!["class".to_string(), "destination".to_string()],
            }),
            endpoint: None,
            identifier,
            storage_policy: None,
        }
    }

    fn create_polygon_test_schema() -> EventSchema {
        let mut identifier = HashMap::new();
        identifier.insert(
            "date".to_string(),
            vec![ValidationRules::DateHandler {
                canonical_format: "%Y%m%d".to_string(),
                required: false,
            }],
        );
        identifier.insert(
            "time".to_string(),
            vec![ValidationRules::TimeHandler { required: false }],
        );
        identifier.insert(
            "polygon".to_string(),
            vec![ValidationRules::PolygonHandler { required: true }],
        );

        EventSchema {
            payload: Some(PayloadConfig {
                allowed_types: vec!["HashMap".to_string()],
                required: true,
            }),
            topic: Some(TopicConfig {
                base: "polygon".to_string(),
                key_order: vec!["date".to_string(), "time".to_string()],
            }),
            endpoint: None,
            identifier,
            storage_policy: None,
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

        let payload = None;
        let result =
            processor.process_request("test_event", &params, &payload, OperationType::Notify);
        assert!(result.is_err());
    }

    #[test]
    fn test_notify_request_all_required_fields_present() {
        let mut schemas = HashMap::new();
        schemas.insert("test_event".to_string(), create_test_schema());
        let registry = NotificationRegistry::from_config(&schemas);
        let processor = NotificationProcessor::new(&registry);

        let mut params = HashMap::new();
        params.insert("class".to_string(), "od".to_string());
        params.insert("destination".to_string(), "SCL".to_string());
        params.insert("optional_field".to_string(), "optional_value".to_string());

        let payload = Some(serde_json::Value::String("test payload".to_string()));
        let result =
            processor.process_request("test_event", &params, &payload, OperationType::Notify);

        assert!(result.is_ok());
        let processing_result = result.unwrap();
        assert_eq!(processing_result.event_type, "test_event");
        assert_eq!(
            processing_result.canonicalized_params.get("class"),
            Some(&"od".to_string())
        );
        assert_eq!(
            processing_result.canonicalized_params.get("destination"),
            Some(&"SCL".to_string())
        );
        assert_eq!(
            processing_result.canonicalized_params.get("optional_field"),
            Some(&"optional_value".to_string())
        );
        assert!(processing_result.spatial_metadata.is_none());
    }

    #[test]
    fn test_watch_request_with_wildcards() {
        let mut schemas = HashMap::new();
        schemas.insert("test_event".to_string(), create_test_schema());
        let registry = NotificationRegistry::from_config(&schemas);
        let processor = NotificationProcessor::new(&registry);

        let mut params = HashMap::new();
        params.insert("class".to_string(), "od".to_string());
        params.insert("destination".to_string(), "SCL".to_string());
        // Missing optional_field should get "*"

        let payload = None;
        let result =
            processor.process_request("test_event", &params, &payload, OperationType::Watch);

        assert!(result.is_ok());
        let processing_result = result.unwrap();
        assert_eq!(
            processing_result.canonicalized_params.get("optional_field"),
            Some(&"*".to_string())
        );
        assert!(processing_result.spatial_metadata.is_none());
    }

    #[test]
    fn test_watch_request_missing_required_field() {
        let mut schemas = HashMap::new();
        schemas.insert("test_event".to_string(), create_test_schema());
        let registry = NotificationRegistry::from_config(&schemas);
        let processor = NotificationProcessor::new(&registry);

        let mut params = HashMap::new();
        params.insert("destination".to_string(), "SCL".to_string());
        // Missing required "class" field

        let payload = None;
        let result =
            processor.process_request("test_event", &params, &payload, OperationType::Watch);

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Required field 'class' missing")
        );
    }

    #[test]
    fn test_replay_request_with_wildcards() {
        let mut schemas = HashMap::new();
        schemas.insert("test_event".to_string(), create_test_schema());
        let registry = NotificationRegistry::from_config(&schemas);
        let processor = NotificationProcessor::new(&registry);

        let mut params = HashMap::new();
        params.insert("class".to_string(), "od".to_string());
        params.insert("destination".to_string(), "SCL".to_string());
        // Missing optional_field should get "*"

        let payload = None;
        let result =
            processor.process_request("test_event", &params, &payload, OperationType::Replay);

        assert!(result.is_ok());
        let processing_result = result.unwrap();
        assert_eq!(
            processing_result.canonicalized_params.get("optional_field"),
            Some(&"*".to_string())
        );
        assert!(processing_result.spatial_metadata.is_none());
    }

    #[test]
    fn test_polygon_notification_with_valid_payload() {
        let mut schemas = HashMap::new();
        schemas.insert("test_polygon".to_string(), create_polygon_test_schema());
        let registry = NotificationRegistry::from_config(&schemas);
        let processor = NotificationProcessor::new(&registry);

        let mut params = HashMap::new();
        params.insert("date".to_string(), "20250706".to_string());
        params.insert("time".to_string(), "1200".to_string());
        params.insert(
            "polygon".to_string(),
            "(52.5,13.4,52.6,13.5,52.5,13.6,52.4,13.5,52.5,13.4)".to_string(),
        );

        // Valid HashMap payload
        let mut payload_map = serde_json::Map::new();
        payload_map.insert(
            "message".to_string(),
            serde_json::Value::String("test".to_string()),
        );
        let payload = Some(serde_json::Value::Object(payload_map));

        let result =
            processor.process_request("test_polygon", &params, &payload, OperationType::Notify);

        assert!(result.is_ok());
        let processing_result = result.unwrap();
        assert!(processing_result.spatial_metadata.is_some());

        let spatial_metadata = processing_result.spatial_metadata.unwrap();
        assert!(!spatial_metadata.bounding_box.is_empty());
        assert!(spatial_metadata.bounding_box.contains(','));
    }

    #[test]
    fn test_polygon_notification_with_invalid_payload_type() {
        let mut schemas = HashMap::new();
        schemas.insert("test_polygon".to_string(), create_polygon_test_schema());
        let registry = NotificationRegistry::from_config(&schemas);
        let processor = NotificationProcessor::new(&registry);

        let mut params = HashMap::new();
        params.insert("date".to_string(), "20250706".to_string());
        params.insert("time".to_string(), "1200".to_string());
        params.insert(
            "polygon".to_string(),
            "(52.5,13.4,52.6,13.5,52.5,13.6,52.4,13.5,52.5,13.4)".to_string(),
        );

        // Invalid String payload when HashMap is required
        let payload = Some(serde_json::Value::String(
            "invalid payload type".to_string(),
        ));

        let result =
            processor.process_request("test_polygon", &params, &payload, OperationType::Notify);

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("payload must be a HashMap/Object")
        );
    }

    #[test]
    fn test_polygon_notification_with_missing_payload() {
        let mut schemas = HashMap::new();
        schemas.insert("test_polygon".to_string(), create_polygon_test_schema());
        let registry = NotificationRegistry::from_config(&schemas);
        let processor = NotificationProcessor::new(&registry);

        let mut params = HashMap::new();
        params.insert("date".to_string(), "20250706".to_string());
        params.insert("time".to_string(), "1200".to_string());
        params.insert(
            "polygon".to_string(),
            "(52.5,13.4,52.6,13.5,52.5,13.6,52.4,13.5,52.5,13.4)".to_string(),
        );

        // Missing payload when polygon field is present
        let payload = None;

        let result =
            processor.process_request("test_polygon", &params, &payload, OperationType::Notify);

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("payload is required and must be a HashMap/Object")
        );
    }

    #[test]
    fn test_polygon_watch_request_no_payload_validation() {
        let mut schemas = HashMap::new();
        schemas.insert("test_polygon".to_string(), create_polygon_test_schema());
        let registry = NotificationRegistry::from_config(&schemas);
        let processor = NotificationProcessor::new(&registry);

        let mut params = HashMap::new();
        params.insert(
            "polygon".to_string(),
            "(52.5,13.4,52.6,13.5,52.5,13.6,52.4,13.5,52.5,13.4)".to_string(),
        );
        // Missing optional date and time fields

        // Watch operations don't validate payload type
        let payload = None;

        let result =
            processor.process_request("test_polygon", &params, &payload, OperationType::Watch);

        assert!(result.is_ok());
        let processing_result = result.unwrap();
        assert_eq!(
            processing_result.canonicalized_params.get("date"),
            Some(&"*".to_string())
        );
        assert_eq!(
            processing_result.canonicalized_params.get("time"),
            Some(&"*".to_string())
        );
        assert!(processing_result.spatial_metadata.is_none()); // Watch doesn't extract spatial metadata
    }

    #[test]
    fn test_polygon_watch_request_accepts_point_without_polygon_identifier() {
        let mut schemas = HashMap::new();
        schemas.insert("test_polygon".to_string(), create_polygon_test_schema());
        let registry = NotificationRegistry::from_config(&schemas);
        let processor = NotificationProcessor::new(&registry);

        let mut params = HashMap::new();
        params.insert("time".to_string(), "1200".to_string());
        params.insert("point".to_string(), "52.55,13.5".to_string());

        let payload = None;
        let result =
            processor.process_request("test_polygon", &params, &payload, OperationType::Watch);

        assert!(result.is_ok());
        let processing_result = result.unwrap();
        assert_eq!(
            processing_result.canonicalized_params.get("polygon"),
            Some(&"*".to_string())
        );
        assert_eq!(
            processing_result.canonicalized_params.get("point"),
            Some(&"52.55,13.5".to_string())
        );
    }

    #[test]
    fn test_polygon_replay_request_accepts_point_without_polygon_identifier() {
        let mut schemas = HashMap::new();
        schemas.insert("test_polygon".to_string(), create_polygon_test_schema());
        let registry = NotificationRegistry::from_config(&schemas);
        let processor = NotificationProcessor::new(&registry);

        let mut params = HashMap::new();
        params.insert("time".to_string(), "1200".to_string());
        params.insert("point".to_string(), "52.55,13.5".to_string());

        let payload = None;
        let result =
            processor.process_request("test_polygon", &params, &payload, OperationType::Replay);

        assert!(result.is_ok());
        let processing_result = result.unwrap();
        assert_eq!(
            processing_result.canonicalized_params.get("polygon"),
            Some(&"*".to_string())
        );
        assert_eq!(
            processing_result.canonicalized_params.get("point"),
            Some(&"52.55,13.5".to_string())
        );
    }

    #[test]
    fn test_generic_processing_notify_empty_values() {
        let registry = NotificationRegistry::new();
        let processor = NotificationProcessor::new(&registry);

        let mut params = HashMap::new();
        params.insert("field1".to_string(), "".to_string()); // Empty value
        params.insert("field2".to_string(), "valid".to_string());

        let payload = None;
        let result =
            processor.process_request("unknown_event", &params, &payload, OperationType::Notify);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("cannot be empty"));
    }

    #[test]
    fn test_generic_processing_notify_valid_values() {
        let registry = NotificationRegistry::new();
        let processor = NotificationProcessor::new(&registry);

        let mut params = HashMap::new();
        params.insert("field1".to_string(), "value1".to_string());
        params.insert("field2".to_string(), "value2".to_string());

        let payload = None;
        let result =
            processor.process_request("unknown_event", &params, &payload, OperationType::Notify);

        assert!(result.is_ok());
        let processing_result = result.unwrap();
        assert_eq!(processing_result.event_type, "unknown_event");
        assert_eq!(
            processing_result.canonicalized_params.get("field1"),
            Some(&"value1".to_string())
        );
        assert_eq!(
            processing_result.canonicalized_params.get("field2"),
            Some(&"value2".to_string())
        );
        assert!(processing_result.spatial_metadata.is_none());
    }

    #[test]
    fn test_generic_processing_watch_empty_values_allowed() {
        let registry = NotificationRegistry::new();
        let processor = NotificationProcessor::new(&registry);

        let mut params = HashMap::new();
        params.insert("field1".to_string(), "".to_string()); // Empty value allowed for watch
        params.insert("field2".to_string(), "valid".to_string());

        let payload = None;
        let result =
            processor.process_request("unknown_event", &params, &payload, OperationType::Watch);

        assert!(result.is_ok());
        let processing_result = result.unwrap();
        assert_eq!(
            processing_result.canonicalized_params.get("field1"),
            Some(&"".to_string())
        );
        assert_eq!(
            processing_result.canonicalized_params.get("field2"),
            Some(&"valid".to_string())
        );
    }

    #[test]
    fn test_validation_rule_not_found() {
        let mut identifier = HashMap::new();
        identifier.insert("field".to_string(), vec![]); // Empty rules

        let schema = EventSchema {
            payload: None,
            topic: None,
            endpoint: None,
            identifier,
            storage_policy: None,
        };

        let mut schemas = HashMap::new();
        schemas.insert("test_event".to_string(), schema);
        let registry = NotificationRegistry::from_config(&schemas);
        let processor = NotificationProcessor::new(&registry);

        let mut params = HashMap::new();
        params.insert("field".to_string(), "value".to_string());

        let payload = None;
        let result =
            processor.process_request("test_event", &params, &payload, OperationType::Notify);

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("No validation rules found")
        );
    }

    #[test]
    fn test_payload_extraction_optional_missing() {
        // Create a schema with optional payload configuration
        let mut identifier = HashMap::new();
        identifier.insert(
            "class".to_string(),
            vec![ValidationRules::StringHandler {
                max_length: Some(2),
                required: true,
            }],
        );
        identifier.insert(
            "destination".to_string(),
            vec![ValidationRules::StringHandler {
                max_length: None,
                required: true,
            }],
        );

        let schema = EventSchema {
            payload: Some(PayloadConfig {
                allowed_types: vec!["String".to_string()],
                required: false, // Payload is optional
            }),
            topic: Some(TopicConfig {
                base: "test".to_string(),
                key_order: vec!["class".to_string(), "destination".to_string()],
            }),
            endpoint: None,
            identifier,
            storage_policy: None,
        };

        let mut schemas = HashMap::new();
        schemas.insert("test_event".to_string(), schema);
        let registry = NotificationRegistry::from_config(&schemas);
        let processor = NotificationProcessor::new(&registry);

        let mut params = HashMap::new();
        params.insert("class".to_string(), "od".to_string());
        params.insert("destination".to_string(), "SCL".to_string());

        let payload = None; // Missing optional payload
        let result =
            processor.process_request("test_event", &params, &payload, OperationType::Notify);

        assert!(result.is_ok());
    }

    #[test]
    fn test_topic_generation_with_schema() {
        let mut schemas = HashMap::new();
        schemas.insert("test_event".to_string(), create_test_schema());
        let registry = NotificationRegistry::from_config(&schemas);
        let processor = NotificationProcessor::new(&registry);

        let mut params = HashMap::new();
        params.insert("class".to_string(), "od".to_string());
        params.insert("destination".to_string(), "SCL".to_string());
        params.insert("optional_field".to_string(), "optional_value".to_string());

        let payload = Some(serde_json::Value::String("test payload".to_string()));
        let result =
            processor.process_request("test_event", &params, &payload, OperationType::Notify);

        assert!(result.is_ok());
        let processing_result = result.unwrap();
        assert!(processing_result.topic.starts_with("test."));
        assert!(processing_result.topic.contains("od"));
        assert!(processing_result.topic.contains("SCL"));
    }

    #[test]
    fn test_topic_generation_without_schema() {
        let registry = NotificationRegistry::new();
        let processor = NotificationProcessor::new(&registry);

        let mut params = HashMap::new();
        params.insert("field1".to_string(), "value1".to_string());
        params.insert("field2".to_string(), "value2".to_string());

        let payload = None;
        let result =
            processor.process_request("unknown_event", &params, &payload, OperationType::Notify);

        assert!(result.is_ok());
        let processing_result = result.unwrap();
        assert!(processing_result.topic.starts_with("unknown_event."));
    }
}
