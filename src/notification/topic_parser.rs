// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Topic subject parsing helpers.

use anyhow::{Context, Result, bail};
use std::collections::HashMap;

use crate::configuration::{EventSchema, Settings};
use crate::notification::topic_codec::{decode_subject, decode_subject_base};

/// A logical routing base cannot require escaping or introduce NATS wildcards.
#[derive(Debug, thiserror::Error)]
#[error("topic base must match [A-Za-z0-9][A-Za-z0-9_-]*")]
pub struct InvalidTopicBase;

/// Validate a logical base, not an encoded subject or identifier value.
/// For example, `weather_v2` is valid, but `weather.v2` and `_weather` are not.
pub fn validate_topic_base(base: &str) -> std::result::Result<(), InvalidTopicBase> {
    if base
        .as_bytes()
        .first()
        .is_some_and(u8::is_ascii_alphanumeric)
        && base
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        Ok(())
    } else {
        Err(InvalidTopicBase)
    }
}

/// JetStream names and subject bindings derived from a validated logical base.
pub struct TopicBinding {
    /// Existing ASCII stream names remain uppercase.
    pub stream_name: String,
    /// Subject case remains the configured case.
    pub subject_pattern: String,
}

impl TopicBinding {
    /// Bind a logical base without changing its subject spelling.
    pub fn new(base: &str) -> std::result::Result<Self, InvalidTopicBase> {
        validate_topic_base(base)?;
        Ok(Self {
            stream_name: base.to_ascii_uppercase(),
            subject_pattern: format!("{base}.>"),
        })
    }
}

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
    Ok(TopicBinding::new(&event_type)?.stream_name)
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
        assert!(derive_stream_name_from_topic("diss%2Ev2.FOO").is_err());
        assert!(derive_stream_name_from_topic("diss%252Ev2.FOO").is_err());
    }

    #[test]
    fn logical_base_contract() {
        for base in ["weather", "Weather_v2", "weather-v2", "9", "UPPER"] {
            let binding = TopicBinding::new(base).unwrap();
            assert_eq!(binding.stream_name, base.to_ascii_uppercase());
            assert_eq!(binding.subject_pattern, format!("{base}.>"));
        }
        for base in [
            "", ".", "%", "*", ">", "a b", "a\t", "a\n", "\u{e9}", "_a", "-a", "a.b", "a%2Eb",
        ] {
            assert!(TopicBinding::new(base).is_err(), "{base:?}");
        }
    }
}
