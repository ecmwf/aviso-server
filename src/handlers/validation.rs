use crate::types::NotificationRequest;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RequestParseError {
    #[error("JSON parsing failed: {0}")]
    InvalidJson(serde_json::Error),
    #[error("Request contains unknown fields: {0}")]
    UnknownField(anyhow::Error),
    #[error("Request deserialization failed: {0}")]
    InvalidShape(serde_json::Error),
}

/// Parse and validate request body for known fields
pub fn parse_and_validate_request(body: &[u8]) -> Result<NotificationRequest, RequestParseError> {
    // Parse as JSON value first for field validation
    let json_value: serde_json::Value =
        serde_json::from_slice(body).map_err(RequestParseError::InvalidJson)?;

    // Validate known fields
    NotificationRequest::validate_known_fields(&json_value)
        .map_err(RequestParseError::UnknownField)?;

    // Deserialize to NotificationRequest after validation
    serde_json::from_value(json_value).map_err(RequestParseError::InvalidShape)
}
