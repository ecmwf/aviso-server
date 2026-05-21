// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! High-level notification entry points.

use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;

use super::{NotificationProcessor, NotificationRegistry, OperationType, ProcessingResult};

/// Public facade over registry + processor.
pub struct NotificationHandler {
    registry: NotificationRegistry,
}

impl NotificationHandler {
    /// Create a new notification handler from configuration.
    ///
    /// `strict` controls whether unknown event types are rejected
    /// (true) or accepted via the legacy generic fallback (false).
    pub fn from_config(
        notification_schema: Option<&HashMap<String, crate::configuration::EventSchema>>,
        strict: bool,
    ) -> Self {
        let registry = if let Some(schemas) = notification_schema {
            NotificationRegistry::from_config_with_strict(schemas, strict)
        } else {
            NotificationRegistry::from_config_with_strict(&HashMap::new(), strict)
        };

        Self { registry }
    }

    /// Validate request parameters and build routing topic.
    pub fn process_request(
        &self,
        event_type: &str,
        request_params: &HashMap<String, Value>,
        payload: &Option<serde_json::Value>,
        operation: OperationType,
    ) -> Result<ProcessingResult> {
        let processor = NotificationProcessor::new(&self.registry);
        processor.process_request_with_values(event_type, request_params, payload, operation)
    }

    /// Get all identifier keys defined for an event type.
    pub fn get_identifier_keys(&self, event_type: &str) -> Result<Vec<String>> {
        self.registry.get_identifier_keys(event_type)
    }

    /// Get required identifier keys for an event type.
    pub fn get_required_identifier_keys(&self, event_type: &str) -> Result<Vec<String>> {
        self.registry.get_required_identifier_keys(event_type)
    }

    /// Get full schema map.
    pub fn get_whole_schema(&self) -> &HashMap<String, crate::configuration::EventSchema> {
        self.registry.get_whole_schema()
    }
}
