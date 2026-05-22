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
use crate::notification::topic_parser::{derive_event_type_from_topic, topic_to_request};
use crate::notification::{
    POLYGON_IDENTIFIER_FIELD, SPATIAL_GEOMETRY_METADATA_KEY, decode_subject_for_display,
};
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
        let data = self.build_cloud_event_data(
            &request_params,
            &notification.payload,
            notification.metadata.as_ref(),
        )?;

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
    /// The polygon identifier field (when the schema declares one) is not part
    /// of the NATS subject and therefore is NOT recovered by `topic_to_request`.
    /// We re-attach it from the stored `spatial_geometry` backend header so the
    /// CloudEvent's `data.identifier` matches the identifier the producer sent.
    fn build_cloud_event_data(
        &self,
        identifier_params: &HashMap<String, String>,
        payload: &str,
        metadata: Option<&HashMap<String, String>>,
    ) -> Result<serde_json::Value> {
        let payload_json = self
            .parse_payload_to_json(payload)
            .context("Failed to parse notification payload as JSON")?;

        let mut identifier: HashMap<String, String> = identifier_params.clone();
        if let Some(meta) = metadata
            && let Some(polygon) = meta.get(SPATIAL_GEOMETRY_METADATA_KEY)
        {
            identifier
                .entry(POLYGON_IDENTIFIER_FIELD.to_string())
                .or_insert_with(|| polygon.clone());
        }

        Ok(json!({
            "identifier": identifier,
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
    fn build_cloud_event_data_reinjects_polygon_from_spatial_geometry_metadata() {
        let creator = CloudEventCreator::new("http://test.com".to_string());

        let mut identifier_params = HashMap::new();
        identifier_params.insert("date".to_string(), "20260522".to_string());
        identifier_params.insert("time".to_string(), "1200".to_string());

        let mut metadata = HashMap::new();
        let polygon = "(50.0,10.0,52.0,10.0,52.0,12.0,50.0,12.0,50.0,10.0)";
        metadata.insert(
            SPATIAL_GEOMETRY_METADATA_KEY.to_string(),
            polygon.to_string(),
        );

        let data = creator
            .build_cloud_event_data(&identifier_params, r#"{"hello":"world"}"#, Some(&metadata))
            .expect("data builder must succeed");

        let identifier = data
            .get("identifier")
            .and_then(|v| v.as_object())
            .expect("identifier must be a JSON object");
        assert_eq!(
            identifier.get("date").and_then(|v| v.as_str()),
            Some("20260522")
        );
        assert_eq!(
            identifier.get("time").and_then(|v| v.as_str()),
            Some("1200")
        );
        assert_eq!(
            identifier
                .get(POLYGON_IDENTIFIER_FIELD)
                .and_then(|v| v.as_str()),
            Some(polygon),
            "polygon must be re-injected from spatial_geometry metadata header"
        );
    }

    #[test]
    fn build_cloud_event_data_leaves_identifier_alone_when_no_metadata() {
        let creator = CloudEventCreator::new("http://test.com".to_string());

        let mut identifier_params = HashMap::new();
        identifier_params.insert("class".to_string(), "od".to_string());

        let data = creator
            .build_cloud_event_data(&identifier_params, r#"{}"#, None)
            .expect("data builder must succeed");

        let identifier = data
            .get("identifier")
            .and_then(|v| v.as_object())
            .expect("identifier must be a JSON object");
        assert_eq!(identifier.len(), 1, "no extra fields when no metadata");
        assert!(
            !identifier.contains_key(POLYGON_IDENTIFIER_FIELD),
            "must not invent a polygon field when none was sent"
        );
    }

    #[test]
    fn build_cloud_event_data_ignores_metadata_without_spatial_geometry() {
        let creator = CloudEventCreator::new("http://test.com".to_string());

        let mut identifier_params = HashMap::new();
        identifier_params.insert("class".to_string(), "od".to_string());

        let mut metadata = HashMap::new();
        metadata.insert("some_other_header".to_string(), "value".to_string());

        let data = creator
            .build_cloud_event_data(&identifier_params, r#"{}"#, Some(&metadata))
            .expect("data builder must succeed");

        let identifier = data.get("identifier").and_then(|v| v.as_object()).unwrap();
        assert!(
            !identifier.contains_key(POLYGON_IDENTIFIER_FIELD),
            "must not inject polygon when metadata has no spatial_geometry"
        );
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
