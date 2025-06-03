//! Topic string generation for notification routing
//!
//! The topic builder creates routing strings used by the notification backend
//! to organize and route messages. It supports both schema-driven topic
//! construction and generic fallback for unknown event types.

use anyhow::{Context, Result};
use std::collections::HashMap;

use crate::configuration::EventSchema;

/// Builder for notification topic strings
///
/// Topics are hierarchical strings used for routing notifications in the backend.
/// The format depends on whether a schema is available:
/// - Schema-based: Uses configured base and key order (e.g., "diss.FOO.E1.od.0001")
/// - Generic: Uses event type and sorted parameters (e.g., "unknown.param1.param2")
pub struct TopicBuilder;

impl TopicBuilder {
    /// Build topic string using schema configuration
    ///
    /// Uses the schema's topic configuration to build a structured topic string
    /// with the specified base, separator, and key ordering.
    ///
    /// # Arguments
    /// * `event_type` - The event type name
    /// * `schema` - The schema definition containing topic configuration
    /// * `canonicalized_params` - The validated and canonicalized parameters
    ///
    /// # Returns
    /// * `Ok(String)` - The constructed topic string
    /// * `Err(anyhow::Error)` - Missing required parameter for topic building
    ///
    /// # Example
    /// For a dissemination schema with base "diss" and key_order ["destination", "target"],
    /// parameters {"destination": "FOO", "target": "E1"} would produce "diss.FOO.E1"
    pub fn build_topic_with_schema(
        event_type: &str,
        schema: &EventSchema,
        canonicalized_params: &HashMap<String, String>,
    ) -> Result<String> {
        if let Some(topic_config) = &schema.topic {
            let mut topic_parts = vec![topic_config.base.clone()];

            // Add parameters in the order specified by the schema
            for key in &topic_config.key_order {
                let value = canonicalized_params
                    .get(key)
                    .context(format!("Missing key '{}' for topic building", key))?;
                topic_parts.push(value.clone());
            }

            Ok(topic_parts.join(&topic_config.separator))
        } else {
            // Fallback to generic topic format if no topic config in schema
            Ok(Self::build_generic_topic(event_type, canonicalized_params))
        }
    }

    /// Build generic topic string when no schema is available
    ///
    /// Creates a simple topic format using the event type and all parameter
    /// values in sorted key order. This ensures consistent topic generation
    /// for unknown event types.
    ///
    /// # Arguments
    /// * `event_type` - The event type name
    /// * `canonicalized_params` - The validated parameters
    ///
    /// # Returns
    /// The constructed topic string in format: "{event_type}.{value1}.{value2}..."
    ///
    /// # Example
    /// For event_type "custom" with parameters {"b": "two", "a": "one"},
    /// this would produce "custom.one.two" (sorted by key name)
    pub fn build_generic_topic(
        event_type: &str,
        canonicalized_params: &HashMap<String, String>,
    ) -> String {
        if canonicalized_params.is_empty() {
            return event_type.to_string();
        }

        // Sort keys to ensure consistent topic ordering
        let mut sorted_keys: Vec<_> = canonicalized_params.keys().collect();
        sorted_keys.sort();

        let mut topic_parts = vec![event_type.to_string()];

        // Add parameter values in sorted key order
        for key in sorted_keys {
            if let Some(value) = canonicalized_params.get(key) {
                topic_parts.push(value.clone());
            }
        }

        topic_parts.join(".")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::TopicConfig;

    #[test]
    fn test_generic_topic_building() {
        let mut params = HashMap::new();
        params.insert("class".to_string(), "od".to_string());
        params.insert("stream".to_string(), "enfo".to_string());

        let topic = TopicBuilder::build_generic_topic("mars", &params);
        // Should be sorted by key name: class, stream
        assert_eq!(topic, "mars.od.enfo");
    }

    #[test]
    fn test_empty_params_generic_topic() {
        let params = HashMap::new();
        let topic = TopicBuilder::build_generic_topic("mars", &params);
        assert_eq!(topic, "mars");
    }

    #[test]
    fn test_schema_topic_building() {
        let mut params = HashMap::new();
        params.insert("destination".to_string(), "FOO".to_string());
        params.insert("target".to_string(), "E1".to_string());
        params.insert("class".to_string(), "od".to_string());

        let topic_config = TopicConfig {
            base: "diss".to_string(),
            separator: ".".to_string(),
            key_order: vec![
                "destination".to_string(),
                "target".to_string(),
                "class".to_string(),
            ],
        };

        let schema = EventSchema {
            payload: None,
            topic: Some(topic_config),
            endpoint: None,
            request: HashMap::new(),
        };

        let topic =
            TopicBuilder::build_topic_with_schema("dissemination", &schema, &params).unwrap();
        assert_eq!(topic, "diss.FOO.E1.od");
    }

    #[test]
    fn test_topic_building_with_missing_schema_keys() {
        let mut params = HashMap::new();
        params.insert("destination".to_string(), "FOO".to_string());
        // Missing "target" which is required in key_order

        let topic_config = TopicConfig {
            base: "diss".to_string(),
            separator: ".".to_string(),
            key_order: vec!["destination".to_string(), "target".to_string()],
        };

        let schema = EventSchema {
            payload: None,
            topic: Some(topic_config),
            endpoint: None,
            request: HashMap::new(),
        };

        let result = TopicBuilder::build_topic_with_schema("dissemination", &schema, &params);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Missing key 'target'")
        );
    }

    #[test]
    fn test_topic_building_with_special_characters() {
        let mut params = HashMap::new();
        params.insert("field1".to_string(), "value-with-dash".to_string());
        params.insert("field2".to_string(), "value_with_underscore".to_string());
        params.insert("field3".to_string(), "value.with.dots".to_string());

        let topic = TopicBuilder::build_generic_topic("test", &params);
        // Should preserve special characters in values
        assert!(topic.contains("value-with-dash"));
        assert!(topic.contains("value_with_underscore"));
        assert!(topic.contains("value.with.dots"));
    }

    #[test]
    fn test_topic_consistency_across_operations() {
        let mut params = HashMap::new();
        params.insert("class".to_string(), "od".to_string());
        params.insert("destination".to_string(), "SCL".to_string());

        // Build topic multiple times with same parameters
        let topic1 = TopicBuilder::build_generic_topic("dissemination", &params);
        let topic2 = TopicBuilder::build_generic_topic("dissemination", &params);
        let topic3 = TopicBuilder::build_generic_topic("dissemination", &params);

        assert_eq!(topic1, topic2);
        assert_eq!(topic2, topic3);
    }

    #[test]
    fn test_custom_separator_in_schema() {
        let mut params = HashMap::new();
        params.insert("a".to_string(), "1".to_string());
        params.insert("b".to_string(), "2".to_string());

        let topic_config = TopicConfig {
            base: "test".to_string(),
            separator: "/".to_string(), // Custom separator
            key_order: vec!["a".to_string(), "b".to_string()],
        };

        let schema = EventSchema {
            payload: None,
            topic: Some(topic_config),
            endpoint: None,
            request: HashMap::new(),
        };

        let topic = TopicBuilder::build_topic_with_schema("test", &schema, &params).unwrap();
        assert_eq!(topic, "test/1/2");
    }

    #[test]
    fn test_single_parameter_topic() {
        let mut params = HashMap::new();
        params.insert("only_param".to_string(), "only_value".to_string());

        let topic = TopicBuilder::build_generic_topic("single", &params);
        assert_eq!(topic, "single.only_value");
    }

    #[test]
    fn test_large_number_of_parameters() {
        let mut params = HashMap::new();
        for i in 0..100 {
            params.insert(format!("param{:03}", i), format!("value{}", i));
        }

        let topic = TopicBuilder::build_generic_topic("large", &params);

        // Should start with event type
        assert!(topic.starts_with("large."));

        // Should contain all parameters (sorted by key)
        assert!(topic.contains("value0"));
        assert!(topic.contains("value99"));

        // Should be deterministic
        let topic2 = TopicBuilder::build_generic_topic("large", &params);
        assert_eq!(topic, topic2);
    }
}
