// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Schema registry for notification processing.

use anyhow::Result;
use std::collections::HashMap;

use crate::configuration::EventSchema;

/// In-memory lookup map for event schemas.
#[derive(Clone)]
pub struct NotificationRegistry {
    /// Event type -> schema.
    schemas: HashMap<String, EventSchema>,
}

impl Default for NotificationRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl NotificationRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            schemas: HashMap::new(),
        }
    }

    /// Build registry from config schemas.
    pub fn from_config(schemas: &HashMap<String, EventSchema>) -> Self {
        Self {
            schemas: schemas.clone(),
        }
    }

    /// Get schema for event type, if present.
    pub fn get_schema(&self, event_type: &str) -> Option<&EventSchema> {
        self.schemas.get(event_type)
    }

    /// Check schema presence for event type.
    pub fn has_schema(&self, event_type: &str) -> bool {
        self.schemas.contains_key(event_type)
    }

    /// List configured event types.
    pub fn get_schema_names(&self) -> Vec<String> {
        self.schemas.keys().cloned().collect()
    }

    /// Get identifier field names for an event type.
    /// Unknown event types return an empty list.
    pub fn get_identifier_keys(&self, event_type: &str) -> Result<Vec<String>> {
        if let Some(schema) = self.get_schema(event_type) {
            Ok(schema.identifier.keys().cloned().collect())
        } else {
            Ok(Vec::new())
        }
    }

    /// Get required identifier field names for an event type.
    pub fn get_required_identifier_keys(&self, event_type: &str) -> Result<Vec<String>> {
        if let Some(schema) = self.get_schema(event_type) {
            let required_keys: Vec<String> = schema
                .identifier
                .iter()
                .filter_map(|(key, field_config)| {
                    let is_required = field_config.is_required();
                    if is_required { Some(key.clone()) } else { None }
                })
                .collect();
            Ok(required_keys)
        } else {
            Ok(Vec::new())
        }
    }

    /// Get the full schema map.
    pub fn get_whole_schema(&self) -> &HashMap<String, EventSchema> {
        &self.schemas
    }
}
