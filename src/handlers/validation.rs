use crate::types::NotificationRequest;
use anyhow::{Result, anyhow};
use serde_json;

/// Parse and validate request body for known fields
pub fn parse_and_validate_request(body: &[u8]) -> Result<NotificationRequest> {
    // Parse as JSON value first for field validation
    let json_value: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| anyhow!("JSON parsing failed: {}", e))?;

    // Validate known fields
    NotificationRequest::validate_known_fields(&json_value)
        .map_err(|e| anyhow!("Request contains unknown fields: {}", e))?;

    // Deserialize to NotificationRequest after validation
    serde_json::from_value(json_value).map_err(|e| anyhow!("Request deserialization failed: {}", e))
}
