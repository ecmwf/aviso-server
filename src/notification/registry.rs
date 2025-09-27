//! Schema registry for notification validation rules
//!
//! The registry manages all configured schemas and provides query methods
//! for retrieving validation rules, required fields, and schema metadata.

use anyhow::Result;
use std::collections::HashMap;

use crate::configuration::EventSchema;

/// Registry that holds all notification schemas and validation rules
///
/// The registry acts as a centralized store for all schema definitions,
/// providing efficient lookup and query capabilities. It handles both
/// configured schemas and graceful fallback for unknown event types.
#[derive(Clone)]
pub struct NotificationRegistry {
    /// Map of event type names to their complete schema definitions
    schemas: HashMap<String, EventSchema>,
}

impl Default for NotificationRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl NotificationRegistry {
    /// Create a new empty registry
    ///
    /// An empty registry will use generic validation for all event types,
    /// which provides basic non-empty string validation for notify operations
    /// and accepts any values for watch operations.
    pub fn new() -> Self {
        Self {
            schemas: HashMap::new(),
        }
    }

    /// Create a new registry from configuration schemas
    ///
    /// # Arguments
    /// * `schemas` - HashMap of event type names to schema definitions
    ///
    /// # Returns
    /// A new registry populated with the provided schemas
    pub fn from_config(schemas: &HashMap<String, EventSchema>) -> Self {
        Self {
            schemas: schemas.clone(),
        }
    }

    /// Get schema definition for a specific event type
    ///
    /// # Arguments
    /// * `event_type` - The event type to look up
    ///
    /// # Returns
    /// * `Some(&EventSchema)` - Schema found for this event type
    /// * `None` - No schema configured for this event type
    pub fn get_schema(&self, event_type: &str) -> Option<&EventSchema> {
        self.schemas.get(event_type)
    }

    /// Check if a schema exists for the given event type
    ///
    /// # Arguments
    /// * `event_type` - The event type to check
    ///
    /// # Returns
    /// `true` if a schema is configured, `false` otherwise
    pub fn has_schema(&self, event_type: &str) -> bool {
        self.schemas.contains_key(event_type)
    }

    /// Get all available schema names
    ///
    /// # Returns
    /// Vector of all configured event type names
    pub fn get_schema_names(&self) -> Vec<String> {
        self.schemas.keys().cloned().collect()
    }

    /// Get all identifier field names for a specific event type
    ///
    /// # Arguments
    /// * `event_type` - The event type to query
    ///
    /// # Returns
    /// * `Ok(Vec<String>)` - List of all field names defined in the schema
    /// * `Err(anyhow::Error)` - Only if schema exists but is malformed
    ///
    /// For unknown event types, returns an empty list (generic processing).
    pub fn get_identifier_keys(&self, event_type: &str) -> Result<Vec<String>> {
        if let Some(schema) = self.get_schema(event_type) {
            Ok(schema.identifier.keys().cloned().collect())
        } else {
            // If no schema exists, return empty list for generic handling
            Ok(Vec::new())
        }
    }

    /// Get required identifier field names for a specific event type
    ///
    /// # Arguments
    /// * `event_type` - The event type to query
    ///
    /// # Returns
    /// * `Ok(Vec<String>)` - List of field names marked as required
    /// * `Err(anyhow::Error)` - Only if schema exists but is malformed
    ///
    /// A field is considered required if any of its validation rules
    /// has `required: true`.
    pub fn get_required_identifier_keys(&self, event_type: &str) -> Result<Vec<String>> {
        if let Some(schema) = self.get_schema(event_type) {
            let required_keys: Vec<String> = schema
                .identifier
                .iter()
                .filter_map(|(key, rules)| {
                    // Check if any rule marks this field as required
                    let is_required = rules.iter().any(|rule| rule.is_required());
                    if is_required { Some(key.clone()) } else { None }
                })
                .collect();
            Ok(required_keys)
        } else {
            // If no schema exists, no fields are specifically required
            Ok(Vec::new())
        }
    }

    /// Get the complete schema configuration
    ///
    /// # Returns
    /// Reference to the internal schema HashMap
    pub fn get_whole_schema(&self) -> &HashMap<String, EventSchema> {
        &self.schemas
    }
}
