//! CloudEvent conversion utilities
//!
//! This module provides utilities for converting CloudEvents to different formats
//! and extracting information needed for routing, storage, and processing.

use anyhow::{Context, Result};
use cloudevents::{AttributesReader, Event};
use serde_json::Value;

/// CloudEvent conversion utilities for storage, routing, and response generation
///
/// This converter handles the transformation of CloudEvents into formats suitable for:
/// - Storage in the notification backend
/// - Topic generation for message routing
/// - Data extraction for business logic processing
/// - Acknowledgment event generation
pub struct CloudEventConverter;

impl CloudEventConverter {
    /// Convert CloudEvent to a storage-friendly JSON string format
    ///
    /// # Arguments
    /// * `event` - The CloudEvent to serialize
    ///
    /// # Returns
    /// * `Ok(String)` - JSON representation suitable for storage
    /// * `Err(anyhow::Error)` - Serialization failed
    ///
    /// The serialized format preserves all CloudEvent attributes and data,
    /// allowing for complete reconstruction of the original event.
    pub fn serialize_for_storage(event: &Event) -> Result<String> {
        serde_json::to_string(event)
            .context("Failed to serialize CloudEvent to JSON for storage - event may contain non-serializable data")
    }

    /// Extract structured data from CloudEvent as JSON Value
    ///
    /// # Arguments
    /// * `event` - The CloudEvent to extract data from
    ///
    /// # Returns
    /// * `Some(Value)` - Successfully extracted JSON data
    /// * `None` - No data present or data is not JSON-compatible
    ///
    /// This method handles different data formats:
    /// - JSON data: Returned directly as serde_json::Value
    /// - String data: Parsed as JSON if valid, None otherwise
    /// - Binary data: Not supported, returns None
    pub fn extract_data_as_json(event: &Event) -> Option<Value> {
        event.data().and_then(|data| match data {
            cloudevents::Data::Json(json_value) => {
                tracing::debug!(
                    event_id = %event.id(),
                    "Extracted JSON data from CloudEvent"
                );
                Some(json_value.clone())
            }
            cloudevents::Data::String(s) => match serde_json::from_str(s) {
                Ok(json_value) => {
                    tracing::debug!(
                        event_id = %event.id(),
                        "Parsed string data as JSON from CloudEvent"
                    );
                    Some(json_value)
                }
                Err(e) => {
                    tracing::warn!(
                        event_id = %event.id(),
                        error = %e,
                        "Failed to parse string data as JSON"
                    );
                    None
                }
            },
            cloudevents::Data::Binary(_) => {
                tracing::debug!(
                    event_id = %event.id(),
                    "Binary data not supported for JSON extraction"
                );
                None
            }
        })
    }
}
