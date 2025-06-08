use cloudevents::Event;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
#[serde(untagged)]
pub enum PayloadType {
    String(String),
    CloudEvent(Event),
    HashMap(HashMap<String, String>),
}

impl PayloadType {
    /// Get the type name for schema validation
    pub fn type_name(&self) -> &'static str {
        match self {
            PayloadType::String(_) => "String",
            PayloadType::CloudEvent(_) => "CloudEvent",
            PayloadType::HashMap(_) => "HashMap",
        }
    }
}

/// Notification request structure used by both /notification and /watch endpoints
#[derive(Debug, Deserialize, Serialize)]
pub struct NotificationRequest {
    /// Event type for schema lookup and validation
    pub event_type: String,
    /// Request parameters to validate against schema
    pub request: HashMap<String, String>,
    /// Optional message ID for /watch endpoint correlation
    #[serde(default)]
    pub from_id: Option<String>,
    /// Optional date filter for /watch endpoint
    #[serde(default)]
    pub from_date: Option<String>,
    /// Payload with flexible type based on schema configuration
    #[serde(default)]
    pub payload: Option<PayloadType>,
}

impl NotificationRequest {
    /// Get all valid field names for this struct
    pub fn all_field_names() -> Vec<&'static str> {
        vec!["event_type", "request", "from_id", "from_date", "payload"]
    }

    /// Get all valid field names as strings
    pub fn all_field_strings() -> Vec<&'static str> {
        Self::all_field_names()
    }

    /// Check if a field name is valid
    pub fn is_valid_field(field_name: &str) -> bool {
        Self::all_field_names().contains(&field_name)
    }

    /// Validate that the request contains only known fields
    pub fn validate_known_fields(value: &serde_json::Value) -> Result<(), String> {
        if let Some(obj) = value.as_object() {
            for key in obj.keys() {
                if !Self::is_valid_field(key) {
                    return Err(format!(
                        "Unknown field '{}' in request. Allowed fields: {:?}",
                        key,
                        Self::all_field_names()
                    ));
                }
            }
        }
        Ok(())
    }
}
