//! CloudEvents processing module for Aviso server
//!
//! This module provides comprehensive CloudEvents 1.0 specification support including:
//! - Parsing and validation of incoming CloudEvents from JSON
//! - Application of default values for missing optional fields
//! - Conversion utilities for storage and routing

pub mod conversion;
pub mod defaults;
pub mod handler;
pub mod validation;

use anyhow::{Context, Result};
use cloudevents::Event;
use serde_json::Value;

pub use conversion::CloudEventConverter;
pub use defaults::apply_defaults;
pub use validation::CloudEventValidator;

/// Main CloudEvents processor that orchestrates validation, defaults, and conversion
pub struct CloudEventProcessor;

impl CloudEventProcessor {
    /// Parse, validate, and process a CloudEvent from JSON payload
    ///
    /// # Process Flow
    /// - Validate JSON payload is a valid CloudEvent structure
    /// - Parse the JSON payload as a CloudEvent using serde deserialization
    /// - Apply default values to any missing optional fields
    /// - Return the processed CloudEvent
    pub fn process_json_payload(json_payload: Value) -> Result<Event> {
        // Validate JSON structure is a valid CloudEvent before parsing
        CloudEventValidator::validate_json_cloudevent(&json_payload)
            .context("CloudEvent JSON structure validation failed")?;

        // Parse the validated JSON as a CloudEvent
        let event: Event = serde_json::from_value(json_payload)
            .context("Failed to parse validated JSON as CloudEvent")?;

        // Apply defaults to missing optional fields and return
        apply_defaults(event).context("Failed to apply default values to CloudEvent")
    }
}
