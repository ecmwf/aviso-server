//! Topic to request conversion functionality
//!
//! This module provides functionality to convert topic strings back to their
//! original request parameters using the schema configuration. The conversion
//! is straightforward since topic values are already in their canonical form.

use anyhow::{Result, bail};
use std::collections::HashMap;

use crate::configuration::{EventSchema, Settings};

/// Convert a topic string back to request parameters using schema configuration
///
/// This function reverses the topic building process by parsing the topic
/// components and reconstructing the original request parameters. Since topic
/// values are already sanitized and in canonical form, no value conversion
/// is needed - we simply parse the structure.
///
/// # Arguments
/// * `topic` - The topic string to parse (e.g., "diss.FOO.E1.od.0001.g.20190810.0.enfo.1")
/// * `event_type` - The event type for schema lookup ("dissemination", "mars", etc.)
///
/// # Returns
/// * `Ok(HashMap<String, String>)` - Reconstructed request parameters
/// * `Err(anyhow::Error)` - Invalid topic format or missing schema
pub fn topic_to_request(topic: &str, event_type: &str) -> Result<HashMap<String, String>> {
    let schema = Settings::get_global_notification_schema();

    let schema_map = schema
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("No notification schema configured"))?;

    let event_schema = schema_map
        .get(event_type)
        .ok_or_else(|| anyhow::anyhow!("Unknown event type: {}", event_type))?;

    parse_topic_with_schema(topic, event_schema)
}

/// Parse topic using schema configuration
///
/// Uses the schema's topic configuration to parse the topic string back
/// into its component parameters. The parsing follows the key_order defined
/// in the schema to map topic positions to parameter names.
///
/// # Arguments
/// * `topic` - The topic string to parse
/// * `schema` - The event schema containing topic structure definition
///
/// # Returns
/// * `Ok(HashMap<String, String>)` - Parsed request parameters
/// * `Err(anyhow::Error)` - Invalid topic structure or missing configuration
fn parse_topic_with_schema(topic: &str, schema: &EventSchema) -> Result<HashMap<String, String>> {
    let topic_config = schema
        .topic
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("No topic configuration in schema"))?;

    // Split topic by separator (typically ".")
    let topic_parts: Vec<&str> = topic.split(&topic_config.separator).collect();

    // Verify the topic starts with the expected base
    if topic_parts.is_empty() || topic_parts[0] != topic_config.base {
        bail!(
            "Topic '{}' does not match expected base '{}'",
            topic,
            topic_config.base
        );
    }

    let mut request = HashMap::new();

    // Map topic parts to request parameters using key_order from schema
    // Skip the first part (base) and map remaining parts to parameters
    for (i, key) in topic_config.key_order.iter().enumerate() {
        let value_index = i + 1; // Skip base part at index 0

        if value_index < topic_parts.len() {
            let value = topic_parts[value_index];

            // Include non-empty values and skip wildcards
            // Values are already in canonical form, so no conversion needed
            if !value.is_empty() && value != "*" {
                request.insert(key.clone(), value.to_string());
            }
        }
    }

    Ok(request)
}

/// Derive event type from topic string
///
/// Extracts the event type from the topic base (first component before separator).
/// This is useful when you have a topic but need to determine which schema to use.
///
/// # Arguments
/// * `topic` - The topic string to analyze
///
/// # Returns
/// * `String` - The event type derived from topic base
pub fn derive_event_type_from_topic(topic: &str) -> String {
    let first_part = topic.split('.').next().unwrap_or("");
    if first_part.is_empty() {
        "unknown".to_string()
    } else {
        first_part.to_string()
    }
}

/// Derive stream name from topic string
///
/// Converts the topic base to uppercase stream name format, following
/// the convention that stream names are uppercase versions of topic bases.
///
/// # Arguments
/// * `topic` - The topic string to analyze
///
/// # Returns
/// * `String` - The stream name in uppercase format
pub fn derive_stream_name_from_topic(topic: &str) -> String {
    let first_part = topic.split('.').next().unwrap_or("");
    if first_part.is_empty() {
        "UNKNOWN".to_string()
    } else {
        first_part.to_uppercase()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_event_type_from_topic() {
        assert_eq!(derive_event_type_from_topic("diss.FOO.E1"), "diss");
        assert_eq!(derive_event_type_from_topic("mars.od.0001"), "mars");
        assert_eq!(derive_event_type_from_topic(""), "unknown");
        assert_eq!(derive_event_type_from_topic("single"), "single");
    }

    #[test]
    fn test_derive_stream_name_from_topic() {
        assert_eq!(derive_stream_name_from_topic("diss.FOO.E1"), "DISS");
        assert_eq!(derive_stream_name_from_topic("mars.od.0001"), "MARS");
        assert_eq!(derive_stream_name_from_topic(""), "UNKNOWN");
        assert_eq!(derive_stream_name_from_topic("test"), "TEST");
    }
}
