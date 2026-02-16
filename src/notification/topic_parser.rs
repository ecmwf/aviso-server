//! Topic subject parsing helpers.

use anyhow::{Context, Result, bail};
use std::collections::HashMap;

use crate::configuration::{EventSchema, Settings};
use crate::notification::topic_codec::{decode_subject, decode_subject_base};

/// Parse a topic subject back into request parameters using schema key order.
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

/// Parse a schema-based topic subject.
fn parse_topic_with_schema(topic: &str, schema: &EventSchema) -> Result<HashMap<String, String>> {
    let topic_config = schema
        .topic
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("No topic configuration in schema"))?;

    let topic_parts = decode_subject(topic)?;

    // Base token must match schema base for this event type.
    if topic_parts.is_empty() || topic_parts[0] != topic_config.base {
        bail!(
            "Topic '{}' does not match expected base '{}'",
            topic,
            topic_config.base
        );
    }

    let mut request = HashMap::new();

    for (i, key) in topic_config.key_order.iter().enumerate() {
        let value_index = i + 1;

        if value_index < topic_parts.len() {
            let value = &topic_parts[value_index];

            // Wildcards represent omitted optional fields.
            if !value.is_empty() && value != "*" {
                request.insert(key.clone(), value.to_string());
            }
        }
    }

    Ok(request)
}

/// Decode the first subject token as event type.
pub fn derive_event_type_from_topic(topic: &str) -> Result<String> {
    decode_subject_base(topic)
}

/// Derive uppercase stream name from topic base.
pub fn derive_stream_name_from_topic(topic: &str) -> Result<String> {
    let event_type = derive_event_type_from_topic(topic)
        .context("Failed to derive event type for stream name")?;
    Ok(event_type.to_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_event_type_from_topic() {
        assert_eq!(derive_event_type_from_topic("diss.FOO.E1").unwrap(), "diss");
        assert_eq!(
            derive_event_type_from_topic("mars.od.0001").unwrap(),
            "mars"
        );
        assert_eq!(
            derive_event_type_from_topic("diss%2Ev2.FOO").unwrap(),
            "diss.v2"
        );
        assert_eq!(derive_event_type_from_topic("single").unwrap(), "single");
        assert!(derive_event_type_from_topic("").is_err());
    }

    #[test]
    fn test_derive_stream_name_from_topic() {
        assert_eq!(
            derive_stream_name_from_topic("diss.FOO.E1").unwrap(),
            "DISS"
        );
        assert_eq!(
            derive_stream_name_from_topic("mars.od.0001").unwrap(),
            "MARS"
        );
        assert_eq!(derive_stream_name_from_topic("test").unwrap(), "TEST");
        assert!(derive_stream_name_from_topic("").is_err());
    }
}
