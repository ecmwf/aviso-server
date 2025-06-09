//! CloudEvent creation and formatting for watch endpoint
//!
//! This module provides functionality to convert NotificationMessage instances
//! back to CloudEvent format for SSE streaming. It uses the topic parser to
//! reconstruct request parameters and formats them according to CloudEvent spec.

use anyhow::{Context, Result};
use chrono::Utc;
use cloudevents::{EventBuilder, EventBuilderV10};
use serde_json::json;
use std::collections::HashMap;

use crate::configuration::Settings;
use crate::notification::topic_parser::{derive_event_type_from_topic, topic_to_request};
use crate::notification_backend::NotificationMessage;
use cloudevents::AttributesReader;

/// CloudEvent creator for watch endpoint streaming
///
/// This struct provides methods to convert NotificationMessage instances
/// back to CloudEvent format for SSE streaming to clients.
pub struct CloudEventCreator {
    /// Base URL for this server instance (from configuration)
    base_url: String,
}

impl CloudEventCreator {
    /// Create a new CloudEvent creator with server configuration
    ///
    /// # Arguments
    /// * `base_url` - The base URL of this server for CloudEvent source field
    ///
    /// # Returns
    /// A new CloudEvent creator instance
    pub fn new(base_url: String) -> Self {
        Self { base_url }
    }

    /// Create a CloudEvent from a NotificationMessage
    ///
    /// This method converts a stored notification back to CloudEvent format
    /// by reconstructing the original request parameters from the topic and
    /// formatting everything according to CloudEvent specification.
    pub fn create_cloud_event(
        &self,
        notification: &NotificationMessage,
    ) -> Result<cloudevents::Event> {
        let topic_base = derive_event_type_from_topic(&notification.topic);

        // Map topic base to full event type name
        let event_type = map_topic_base_to_event_type(&topic_base);

        // Convert topic back to request parameters using schema
        let request_params = topic_to_request(&notification.topic, &event_type)
            .context("Failed to reconstruct request parameters from topic")?;

        // Build CloudEvent data structure based on schema payload requirements
        let data =
            self.build_cloud_event_data(&request_params, &notification.payload, &event_type)?;

        // Create CloudEvent with all required fields
        let cloud_event = EventBuilderV10::new()
            .id(format!("{}_{}", event_type, notification.sequence))
            .source(&self.base_url)
            .ty(format!("int.ecmwf.aviso.{}", event_type))
            .time(notification.timestamp.unwrap_or_else(Utc::now))
            .data_with_schema(
                "application/json",
                format!("{}/schema/{}", self.base_url, event_type),
                data,
            )
            .build()
            .context("Failed to build CloudEvent")?;

        tracing::debug!(
            event_id = cloud_event.id(),
            event_type = %event_type,
            topic = %notification.topic,
            sequence = notification.sequence,
            "CloudEvent created successfully"
        );

        Ok(cloud_event)
    }

    /// Build CloudEvent data structure based on schema payload requirements
    ///
    /// This method constructs the data field for the CloudEvent, including
    /// the payload only if it's required according to the schema configuration.
    ///
    /// # Arguments
    /// * `request_params` - Reconstructed request parameters
    /// * `payload` - The notification payload string
    /// * `event_type` - The event type for schema lookup
    ///
    /// # Returns
    /// * `Ok(serde_json::Value)` - CloudEvent data structure
    /// * `Err(anyhow::Error)` - Schema lookup or payload parsing failed
    fn build_cloud_event_data(
        &self,
        request_params: &HashMap<String, String>,
        payload: &str,
        event_type: &str,
    ) -> Result<serde_json::Value> {
        // Check if payload is required according to schema
        let include_payload = self.is_payload_required(event_type)?;

        if include_payload {
            // Parse payload back to JSON for CloudEvent data field
            let payload_json = self
                .parse_payload_to_json(payload)
                .context("Failed to parse notification payload as JSON")?;

            Ok(json!({
                "request": request_params,
                "payload": payload_json
            }))
        } else {
            // Payload not required by schema, exclude it
            Ok(json!({
                "request": request_params
            }))
        }
    }

    /// Check if payload is required according to schema configuration
    ///
    /// # Arguments
    /// * `event_type` - The event type to check
    ///
    /// # Returns
    /// * `Ok(bool)` - true if payload is required, false otherwise
    /// * `Err(anyhow::Error)` - Schema lookup failed
    fn is_payload_required(&self, event_type: &str) -> Result<bool> {
        let schema = Settings::get_global_notification_schema();

        let schema_map = schema
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No notification schema configured"))?;

        let event_schema = schema_map
            .get(event_type)
            .ok_or_else(|| anyhow::anyhow!("Unknown event type: {}", event_type))?;

        // Check if payload is configured and required
        let payload_required = event_schema
            .payload
            .as_ref()
            .map(|payload_config| payload_config.required)
            .unwrap_or(false);

        tracing::debug!(
            event_type = %event_type,
            payload_required = payload_required,
            "Checked payload requirement from schema"
        );

        Ok(payload_required)
    }

    /// Parse notification payload string back to JSON value
    ///
    /// The payload is stored as a string in the backend but needs to be
    /// converted back to JSON for the CloudEvent data field.
    ///
    /// # Arguments
    /// * `payload` - The payload string from notification storage
    ///
    /// # Returns
    /// * `Ok(serde_json::Value)` - Parsed JSON value
    /// * `Err(anyhow::Error)` - Invalid JSON format
    fn parse_payload_to_json(&self, payload: &str) -> Result<serde_json::Value> {
        if payload.is_empty() {
            // Empty payload becomes null in JSON
            return Ok(serde_json::Value::Null);
        }

        // Try to parse as JSON first
        match serde_json::from_str::<serde_json::Value>(payload) {
            Ok(json_value) => Ok(json_value),
            Err(_) => {
                // If it's not valid JSON, treat it as a string value
                tracing::debug!(
                    payload_preview = &payload[..payload.len().min(100)],
                    "Payload is not valid JSON, treating as string"
                );
                Ok(serde_json::Value::String(payload.to_string()))
            }
        }
    }

    /// Create a CloudEvent creator from global application settings
    ///
    /// This is a convenience method that reads the base URL from the
    /// global application configuration.
    ///
    /// # Returns
    /// A new CloudEvent creator configured with the global base URL
    pub fn from_global_config() -> Self {
        let app_settings = Settings::get_global_application_settings();
        Self::new(app_settings.base_url.clone())
    }
}

/// Convenience function to create CloudEvent from NotificationMessage
///
/// This is a simplified interface for creating CloudEvents when you don't
/// need to customize the creator configuration.
///
/// # Arguments
/// * `notification` - The notification message to convert
/// * `base_url` - Server base URL for CloudEvent source field
///
/// # Returns
/// * `Ok(cloudevents::Event)` - Formatted CloudEvent
/// * `Err(anyhow::Error)` - Failed to create CloudEvent
pub fn create_cloud_event_from_notification(
    notification: &NotificationMessage,
    base_url: &str,
) -> Result<cloudevents::Event> {
    let creator = CloudEventCreator::new(base_url.to_string());
    creator.create_cloud_event(notification)
}

/// Map topic base to full event type name
///
/// This function maps the topic base (first part of topic) back to the
/// full event type name used in the schema configuration.
///
/// # Arguments
/// * `topic_base` - The base part of the topic (e.g., "diss", "mars")
///
/// # Returns
/// * `String` - The full event type name (e.g., "dissemination", "mars")
fn map_topic_base_to_event_type(topic_base: &str) -> String {
    match topic_base {
        "diss" => "dissemination".to_string(),
        "mars" => "mars".to_string(),
        _ => topic_base.to_string(), // Fallback to base if no mapping found
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn create_test_notification() -> NotificationMessage {
        NotificationMessage {
            sequence: 123,
            topic: "diss.FOO.E1.od.0001.g.20190810.0.enfo.1".to_string(),
            payload: r#"{"test": "data"}"#.to_string(),
            timestamp: Some(Utc::now()),
        }
    }

    #[test]
    fn test_parse_payload_to_json() {
        let creator = CloudEventCreator::new("http://test.com".to_string());

        // Test valid JSON
        let json_payload = r#"{"key": "value"}"#;
        let result = creator.parse_payload_to_json(json_payload).unwrap();
        assert!(result.is_object());

        // Test string payload
        let string_payload = "simple string";
        let result = creator.parse_payload_to_json(string_payload).unwrap();
        assert!(result.is_string());

        // Test empty payload
        let empty_payload = "";
        let result = creator.parse_payload_to_json(empty_payload).unwrap();
        assert!(result.is_null());
    }

    #[test]
    fn test_cloud_event_creation() {
        let notification = create_test_notification();

        // This test would need the global schema to be initialized
        // For now, it's just a structure test
        assert_eq!(notification.sequence, 123);
        assert!(notification.topic.starts_with("diss."));
    }
}
