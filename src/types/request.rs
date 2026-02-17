use crate::notification_backend::replay::StartAt;
use anyhow::{Result, bail};
use aviso_validators::PointHandler;
use chrono::{DateTime, Utc};
use cloudevents::Event;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
#[serde(untagged)]
pub enum PayloadType {
    String(String),
    CloudEvent(Box<Event>),
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

    /// Convert PayloadType to serde_json::Value for processing
    pub fn to_json_value(&self) -> serde_json::Value {
        match self {
            PayloadType::String(s) => serde_json::Value::String(s.clone()),
            PayloadType::HashMap(map) => {
                serde_json::to_value(map).unwrap_or(serde_json::Value::Null)
            }
            PayloadType::CloudEvent(ce) => {
                serde_json::to_value(ce).unwrap_or(serde_json::Value::Null)
            }
        }
    }
}

/// Notification request structure used by both /notification and /watch endpoints
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct NotificationRequest {
    /// Event type for schema lookup and validation
    pub event_type: String,
    /// Request parameters to validate against schema
    pub identifier: HashMap<String, String>,
    /// Optional message ID for /watch endpoint correlation
    #[serde(default)]
    pub from_id: Option<String>,
    /// Optional date filter for /watch endpoint
    #[serde(default)]
    #[schema(example = "2025-09-15T12:00:00Z")]
    pub from_date: Option<String>,
    /// Optional spatial point filter for watch/replay requests ("lat,lon")
    #[serde(default)]
    #[schema(example = "52.5200,13.4050")]
    pub point: Option<String>,
    /// Payload with flexible type based on schema configuration
    #[serde(default)]
    #[schema(value_type = Object, example = json!({"key": "value"}))]
    pub payload: Option<PayloadType>,
}

impl NotificationRequest {
    /// Get all valid field names for this struct
    pub fn all_field_names() -> Vec<&'static str> {
        vec![
            "event_type",
            "identifier",
            "from_id",
            "from_date",
            "point",
            "payload",
        ]
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
    pub fn validate_known_fields(value: &serde_json::Value) -> Result<()> {
        if let Some(obj) = value.as_object() {
            for key in obj.keys() {
                if !Self::is_valid_field(key) {
                    bail!(
                        "Unknown field '{}' in request. Allowed fields: {:?}",
                        key,
                        Self::all_field_names()
                    );
                }
            }
        }
        Ok(())
    }

    /// Validate and parse from_id parameter for watch endpoint
    ///
    /// The from_id parameter specifies a backend-specific sequence number
    /// from which to start replaying historical messages. This must be
    /// a valid unsigned 64-bit integer.
    ///
    /// # Returns
    /// * `Ok(Option<u64>)` - Parsed sequence number or None if not provided
    /// * `Err(anyhow::Error)` - Invalid sequence number format
    pub fn validate_from_id(&self) -> Result<Option<u64>> {
        match &self.from_id {
            Some(id_str) => {
                if id_str.trim().is_empty() {
                    bail!(
                        "from_id cannot be empty. Provide a valid sequence number or omit the field"
                    );
                }

                match id_str.parse::<u64>() {
                    Ok(id) => {
                        tracing::debug!(
                            from_id_str = id_str,
                            from_id_parsed = id,
                            "from_id successfully validated and parsed"
                        );
                        Ok(Some(id))
                    }
                    Err(_) => bail!(
                        "from_id must be a valid positive integer. Got: '{}'. \
                         Valid examples: '1', '123', '9999'",
                        id_str
                    ),
                }
            }
            None => {
                tracing::debug!("from_id not provided - will start from beginning or current time");
                Ok(None)
            }
        }
    }

    /// Validate and parse from_date parameter for watch endpoint
    ///
    /// The from_date parameter specifies a timestamp from which to start
    /// replaying historical messages. This must be a valid RFC3339 datetime
    /// string (ISO 8601 format with timezone).
    ///
    /// # Returns
    /// * `Ok(Option<DateTime<Utc>>)` - Parsed datetime or None if not provided
    /// * `Err(anyhow::Error)` - Invalid datetime format
    pub fn validate_from_date(&self) -> Result<Option<DateTime<Utc>>> {
        match &self.from_date {
            Some(date_str) => {
                if date_str.trim().is_empty() {
                    bail!(
                        "from_date cannot be empty. Provide a valid RFC3339 datetime or omit the field"
                    );
                }

                match DateTime::parse_from_rfc3339(date_str) {
                    Ok(parsed_date) => {
                        let utc_date = parsed_date.with_timezone(&Utc);

                        tracing::debug!(
                            from_date_str = date_str,
                            from_date_parsed = %utc_date,
                            "from_date successfully validated and parsed to UTC"
                        );

                        Ok(Some(utc_date))
                    }
                    Err(parse_error) => bail!(
                        "from_date must be a valid RFC3339 datetime string. Got: '{}'. \
                         Error: {}. \
                         Valid examples: '2025-06-09T13:15:00Z', '2025-06-09T13:15:00+02:00'",
                        date_str,
                        parse_error
                    ),
                }
            }
            None => {
                tracing::debug!(
                    "from_date not provided - will start from beginning or current time"
                );
                Ok(None)
            }
        }
    }

    /// Validate both from_id and from_date parameters together for watch endpoint
    ///
    /// This method validates both parameters and ensures they are not
    /// conflicting. At most one replay parameter may be provided; both may
    /// be omitted to start a live-only watch stream.
    ///
    /// # Returns
    /// * `Ok((Option<u64>, Option<DateTime<Utc>>))` - Parsed values
    /// * `Err(anyhow::Error)` - If any parameter is invalid
    pub fn validate_watch_parameters(&self) -> Result<(Option<u64>, Option<DateTime<Utc>>)> {
        let parsed_id = self.validate_from_id()?;
        let parsed_date = self.validate_from_date()?;

        match (&parsed_id, &parsed_date) {
            (Some(_), Some(_)) => {
                bail!(
                    "Cannot specify both from_id and from_date. Please provide only one replay parameter. \
                     Use from_id for sequence-based replay or from_date for time-based replay."
                );
            }
            (Some(id), None) => {
                tracing::debug!(
                    from_id = id,
                    "Only from_id provided - will replay from sequence number"
                );
            }
            (None, Some(date)) => {
                tracing::debug!(
                    from_date = %date,
                    "Only from_date provided - will replay from timestamp"
                );
            }
            (None, None) => {
                tracing::debug!(
                    "No replay parameters provided - will start with live messages only"
                );
            }
        }

        Ok((parsed_id, parsed_date))
    }

    /// Validate replay parameters and return a typed replay cursor.
    pub fn validate_start_at(&self) -> Result<StartAt> {
        let (parsed_id, parsed_date) = self.validate_watch_parameters()?;
        match (parsed_id, parsed_date) {
            (Some(id), None) => Ok(StartAt::Sequence(id)),
            (None, Some(date)) => Ok(StartAt::Date(date)),
            (None, None) => Ok(StartAt::LiveOnly),
            (Some(_), Some(_)) => unreachable!("validate_watch_parameters enforces exclusivity"),
        }
    }

    /// Validate spatial filter parameters for watch/replay.
    ///
    /// Rules:
    /// - `identifier.polygon` and `point` are mutually exclusive.
    /// - `point` must be a valid "lat,lon" coordinate pair.
    pub fn validate_spatial_filters(&self) -> Result<()> {
        let has_polygon = self.identifier.contains_key("polygon");
        let has_point = self.point.is_some();

        if has_polygon && has_point {
            bail!(
                "Cannot specify both identifier.polygon and point. Provide only one spatial filter."
            );
        }

        if let Some(point) = &self.point {
            PointHandler::parse_point_coordinates(point).map_err(|e| {
                anyhow::anyhow!("point must be a valid 'lat,lon' coordinate pair: {}", e)
            })?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::NotificationRequest;
    use std::collections::HashMap;

    fn base_request() -> NotificationRequest {
        NotificationRequest {
            event_type: "test_polygon".to_string(),
            identifier: HashMap::new(),
            from_id: None,
            from_date: None,
            point: None,
            payload: None,
        }
    }

    #[test]
    fn validate_spatial_filters_accepts_point_without_polygon() {
        let mut request = base_request();
        request.point = Some("12.34,56.78".to_string());
        assert!(request.validate_spatial_filters().is_ok());
    }

    #[test]
    fn validate_spatial_filters_rejects_polygon_and_point_together() {
        let mut request = base_request();
        request
            .identifier
            .insert("polygon".to_string(), "(0,0,0,1,1,1,0,0)".to_string());
        request.point = Some("12.34,56.78".to_string());
        assert!(request.validate_spatial_filters().is_err());
    }

    #[test]
    fn validate_spatial_filters_rejects_invalid_point() {
        let mut request = base_request();
        request.point = Some("not-a-point".to_string());
        assert!(request.validate_spatial_filters().is_err());
    }
}
