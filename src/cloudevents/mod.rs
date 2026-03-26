// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! CloudEvent creation and formatting for watch endpoint
//!
//! This module provides functionality to convert NotificationMessage instances
//! back to CloudEvent format for SSE streaming. It uses the topic parser to
//! reconstruct request parameters and formats them according to CloudEvent spec.

use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use cloudevents::{EventBuilder, EventBuilderV10};
use serde_json::json;
use std::collections::HashMap;

use crate::configuration::Settings;
use crate::notification::decode_subject_for_display;
use crate::notification::topic_parser::{derive_event_type_from_topic, topic_to_request};
use crate::notification_backend::NotificationMessage;
use cloudevents::AttributesReader;
use tracing::debug;

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
        let topic_base = derive_event_type_from_topic(&notification.topic)
            .context("Failed to extract topic base from notification topic")?;

        // Map topic base to full event type name
        let event_type = find_event_type_from_topic_base(&topic_base)
            .context("Failed to determine event type from topic")?;

        // Convert topic back to request parameters using schema
        let request_params = topic_to_request(&notification.topic, &event_type)
            .context("Failed to reconstruct request parameters from topic")?;

        // Build CloudEvent data structure with canonical payload JSON.
        let data = self.build_cloud_event_data(&request_params, &notification.payload)?;

        // Create CloudEvent with all required fields
        let cloud_event = EventBuilderV10::new()
            .id(format!("{}@{}", event_type, notification.sequence))
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

        debug!(
            event_id = cloud_event.id(),
            event_type = %event_type,
            topic = %decode_subject_for_display(&notification.topic),
            sequence = notification.sequence,
            "CloudEvent created successfully"
        );

        Ok(cloud_event)
    }

    /// Build CloudEvent data structure.
    ///
    /// # Arguments
    /// * `request_params` - Reconstructed request parameters
    /// * `payload` - The notification payload string
    ///
    /// # Returns
    /// * `Ok(serde_json::Value)` - CloudEvent data structure
    /// * `Err(anyhow::Error)` - Payload parsing failed
    fn build_cloud_event_data(
        &self,
        identifier_params: &HashMap<String, String>,
        payload: &str,
    ) -> Result<serde_json::Value> {
        let payload_json = self
            .parse_payload_to_json(payload)
            .context("Failed to parse notification payload as JSON")?;

        Ok(json!({
            "identifier": identifier_params,
            "payload": payload_json
        }))
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
                // Keep replay resilient for legacy/plain payloads that were stored
                // before the strict JSON payload contract.
                debug!(
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

/// Find event type from topic base using schema configuration
///
/// This function searches through all configured schemas to find which
/// event type has a topic base matching the given topic base.
fn find_event_type_from_topic_base(topic_base: &str) -> Result<String> {
    let schema = Settings::get_global_notification_schema();

    let schema_map = schema
        .as_ref()
        .ok_or_else(|| anyhow!("No notification schema configured"))?;

    // Search through all schemas to find matching topic base
    for (event_type, event_schema) in schema_map {
        if let Some(topic_config) = &event_schema.topic
            && topic_config.base == topic_base
        {
            debug!(
                topic_base = %topic_base,
                event_type = %event_type,
                "Found event type for topic base using schema"
            );
            return Ok(event_type.clone());
        }
    }

    bail!("No event type found for topic base: {}", topic_base)
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
            metadata: None,
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
