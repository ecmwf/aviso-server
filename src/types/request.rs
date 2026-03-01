use crate::notification_backend::replay::StartAt;
use anyhow::{Result, bail};
use aviso_validators::PointHandler;
use chrono::{DateTime, NaiveDateTime, Utc};
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
    /// Payload with flexible type based on schema configuration
    #[serde(default)]
    #[schema(value_type = Object, example = json!({"key": "value"}))]
    pub payload: Option<PayloadType>,
}

impl NotificationRequest {
    // Accepted examples:
    // - valid: "2026-02-25T18:58:23Z", "2026-02-25 18:58:23", "+1740509903", "1740509903710"
    // - invalid: "2026-02-25", "not-a-date", "-1" (negative unix timestamps are rejected)
    fn parse_from_date_flexible(date_str: &str) -> Result<DateTime<Utc>> {
        let trimmed = date_str.trim();

        if let Some(stripped) = trimmed.strip_prefix('-')
            && !stripped.is_empty()
            && stripped.chars().all(|c| c.is_ascii_digit())
        {
            bail!("negative unix timestamps are not supported: '{}'", trimmed);
        }

        let unsigned_numeric = trimmed.strip_prefix('+').unwrap_or(trimmed);
        if !unsigned_numeric.is_empty() && unsigned_numeric.chars().all(|c| c.is_ascii_digit()) {
            let significant_digits = unsigned_numeric.trim_start_matches('0');
            let digit_count = if significant_digits.is_empty() {
                1
            } else {
                significant_digits.len()
            };
            let value = unsigned_numeric.parse::<i64>().map_err(|e| {
                anyhow::anyhow!(
                    "failed to parse unix timestamp '{}': {}",
                    unsigned_numeric,
                    e
                )
            })?;

            // Numeric inputs use a stable digit-count rule:
            // - up to 11 digits: unix seconds
            // - 12+ digits: unix milliseconds
            // Leading zeros are ignored for classification.
            // This keeps far-future 11-digit seconds valid and correctly treats
            // early 12-digit millisecond epochs (for example year 2000) as millis.
            if digit_count <= 11 {
                return DateTime::<Utc>::from_timestamp(value, 0).ok_or_else(|| {
                    anyhow::anyhow!("invalid unix seconds timestamp '{}'", unsigned_numeric)
                });
            }

            let seconds = value / 1000;
            let millis_remainder = (value % 1000) as u32;
            return DateTime::<Utc>::from_timestamp(seconds, millis_remainder * 1_000_000)
                .ok_or_else(|| {
                    anyhow::anyhow!("invalid unix milliseconds timestamp '{}'", unsigned_numeric)
                });
        }

        if let Ok(parsed) = DateTime::parse_from_rfc3339(trimmed) {
            return Ok(parsed.with_timezone(&Utc));
        }

        for fmt in ["%Y-%m-%d %H:%M:%S%:z", "%Y-%m-%d %H:%M:%S%.f%:z"] {
            if let Ok(parsed) = DateTime::parse_from_str(trimmed, fmt) {
                return Ok(parsed.with_timezone(&Utc));
            }
        }

        for fmt in [
            "%Y-%m-%dT%H:%M:%S",
            "%Y-%m-%dT%H:%M:%S%.f",
            "%Y-%m-%d %H:%M:%S",
            "%Y-%m-%d %H:%M:%S%.f",
        ] {
            if let Ok(parsed) = NaiveDateTime::parse_from_str(trimmed, fmt) {
                return Ok(parsed.and_utc());
            }
        }

        bail!(
            "expected ISO-8601 datetime (RFC3339/space-separated with optional timezone) \
             or unix epoch seconds/milliseconds"
        );
    }

    /// Get all valid field names for this struct
    pub fn all_field_names() -> Vec<&'static str> {
        vec![
            "event_type",
            "identifier",
            "from_id",
            "from_date",
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

    /// Validate and parse from_date parameter for watch endpoint.
    ///
    /// Accepted values include RFC3339 datetimes, space-separated datetimes
    /// (with optional timezone), and unix epoch seconds/milliseconds.
    ///
    /// # Returns
    /// * `Ok(Option<DateTime<Utc>>)` - Parsed datetime or None if not provided
    /// * `Err(anyhow::Error)` - Invalid datetime format
    pub fn validate_from_date(&self) -> Result<Option<DateTime<Utc>>> {
        match &self.from_date {
            Some(date_str) => {
                if date_str.trim().is_empty() {
                    bail!(
                        "from_date cannot be empty. Provide a valid datetime/timestamp or omit the field"
                    );
                }

                let utc_date = Self::parse_from_date_flexible(date_str).map_err(|parse_error| {
                    anyhow::anyhow!(
                        "from_date must be a valid datetime/timestamp. Got: '{}'. Error: {}. \
                         Valid examples: '2025-06-09T13:15:00Z', '2025-06-09 13:15:00+00:00', \
                         '2025-06-09 13:15:00', '1740509903', '1740509903710'",
                        date_str,
                        parse_error
                    )
                })?;

                tracing::debug!(
                    from_date_str = date_str,
                    from_date_parsed = %utc_date,
                    "from_date successfully validated and parsed to UTC"
                );

                Ok(Some(utc_date))
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
    /// - `identifier.polygon` and `identifier.point` are mutually exclusive.
    /// - `identifier.point` must be a valid "lat,lon" coordinate pair.
    pub fn validate_spatial_filters(&self) -> Result<()> {
        let has_polygon = self.identifier.contains_key("polygon");
        let point = self.identifier.get("point");
        let has_point = point.is_some();

        if has_polygon && has_point {
            bail!(
                "Cannot specify both identifier.polygon and identifier.point. Provide only one spatial filter."
            );
        }

        if let Some(point) = point {
            PointHandler::parse_point_coordinates(point).map_err(|e| {
                anyhow::anyhow!(
                    "identifier.point must be a valid 'lat,lon' coordinate pair: {}",
                    e
                )
            })?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::NotificationRequest;
    use chrono::{DateTime, Utc};
    use std::collections::HashMap;

    fn base_request() -> NotificationRequest {
        NotificationRequest {
            event_type: "test_polygon".to_string(),
            identifier: HashMap::new(),
            from_id: None,
            from_date: None,
            payload: None,
        }
    }

    #[test]
    fn validate_spatial_filters_accepts_point_without_polygon() {
        let mut request = base_request();
        request
            .identifier
            .insert("point".to_string(), "12.34,56.78".to_string());
        assert!(request.validate_spatial_filters().is_ok());
    }

    #[test]
    fn validate_spatial_filters_rejects_polygon_and_point_together() {
        let mut request = base_request();
        request
            .identifier
            .insert("polygon".to_string(), "(0,0,0,1,1,1,0,0)".to_string());
        request
            .identifier
            .insert("point".to_string(), "12.34,56.78".to_string());
        assert!(request.validate_spatial_filters().is_err());
    }

    #[test]
    fn validate_spatial_filters_rejects_invalid_point() {
        let mut request = base_request();
        request
            .identifier
            .insert("point".to_string(), "not-a-point".to_string());
        assert!(request.validate_spatial_filters().is_err());
    }

    #[test]
    fn validate_from_date_accepts_rfc3339_and_space_separated_values() {
        let mut request = base_request();
        request.from_date = Some("2025-06-09T13:15:00+02:00".to_string());
        let parsed = request
            .validate_from_date()
            .expect("from_date should parse")
            .expect("from_date should be present");
        let expected = DateTime::parse_from_rfc3339("2025-06-09T11:15:00Z")
            .expect("expected timestamp should parse")
            .with_timezone(&Utc);
        assert_eq!(parsed, expected);

        request.from_date = Some("2025-06-09 13:15:00+00:00".to_string());
        let parsed_space_tz = request
            .validate_from_date()
            .expect("space-separated with timezone should parse")
            .expect("from_date should be present");
        assert_eq!(
            parsed_space_tz,
            DateTime::parse_from_rfc3339("2025-06-09T13:15:00Z")
                .expect("expected timestamp should parse")
                .with_timezone(&Utc)
        );
    }

    #[test]
    fn validate_from_date_accepts_naive_datetime_as_utc() {
        let mut request = base_request();
        request.from_date = Some("2025-06-09 13:15:00".to_string());
        let parsed = request
            .validate_from_date()
            .expect("naive datetime should parse")
            .expect("from_date should be present");
        assert_eq!(
            parsed,
            DateTime::parse_from_rfc3339("2025-06-09T13:15:00Z")
                .expect("expected timestamp should parse")
                .with_timezone(&Utc)
        );
    }

    #[test]
    fn validate_from_date_accepts_unix_epoch_seconds_and_milliseconds() {
        let mut request = base_request();
        request.from_date = Some("1740509903".to_string());
        let parsed_seconds = request
            .validate_from_date()
            .expect("unix seconds should parse")
            .expect("from_date should be present");
        assert_eq!(parsed_seconds.timestamp(), 1_740_509_903);
        assert_eq!(parsed_seconds.timestamp_subsec_millis(), 0);

        request.from_date = Some("1740509903710".to_string());
        let parsed_millis = request
            .validate_from_date()
            .expect("unix milliseconds should parse")
            .expect("from_date should be present");
        assert_eq!(parsed_millis.timestamp(), 1_740_509_903);
        assert_eq!(parsed_millis.timestamp_subsec_millis(), 710);
    }

    #[test]
    fn validate_from_date_accepts_positive_unix_timestamp_with_plus_prefix() {
        let mut request = base_request();
        request.from_date = Some("+1740509903".to_string());
        let parsed = request
            .validate_from_date()
            .expect("plus-prefixed unix seconds should parse")
            .expect("from_date should be present");
        assert_eq!(parsed.timestamp(), 1_740_509_903);
        assert_eq!(parsed.timestamp_subsec_millis(), 0);
    }

    #[test]
    fn validate_from_date_accepts_far_future_unix_seconds() {
        let mut request = base_request();
        request.from_date = Some("10000000000".to_string());
        let parsed = request
            .validate_from_date()
            .expect("11-digit unix seconds should parse as seconds")
            .expect("from_date should be present");
        assert_eq!(parsed.timestamp(), 10_000_000_000);
        assert_eq!(parsed.timestamp_subsec_millis(), 0);
    }

    #[test]
    fn validate_from_date_uses_millis_at_threshold_boundary() {
        let mut request = base_request();
        request.from_date = Some("1000000000000".to_string());
        let parsed = request
            .validate_from_date()
            .expect("12-digit unix timestamp should parse as milliseconds")
            .expect("from_date should be present");
        assert_eq!(parsed.timestamp(), 1_000_000_000);
        assert_eq!(parsed.timestamp_subsec_millis(), 0);
    }

    #[test]
    fn validate_from_date_accepts_pre_2001_unix_millis() {
        let mut request = base_request();
        request.from_date = Some("946684800000".to_string());
        let parsed = request
            .validate_from_date()
            .expect("12-digit pre-2001 unix millis should parse as milliseconds")
            .expect("from_date should be present");
        assert_eq!(parsed.timestamp(), 946_684_800);
        assert_eq!(parsed.timestamp_subsec_millis(), 0);
    }

    #[test]
    fn validate_from_date_interprets_leading_zero_numeric_values_by_magnitude() {
        let mut request = base_request();
        request.from_date = Some("00000000001740509903".to_string());
        let parsed = request
            .validate_from_date()
            .expect("leading-zero numeric value should parse as unix seconds")
            .expect("from_date should be present");
        assert_eq!(parsed.timestamp(), 1_740_509_903);
        assert_eq!(parsed.timestamp_subsec_millis(), 0);
    }

    #[test]
    fn validate_from_date_rejects_negative_unix_timestamps() {
        let mut request = base_request();
        request.from_date = Some("-1".to_string());
        assert!(request.validate_from_date().is_err());
    }

    #[test]
    fn validate_from_date_accepts_fractional_seconds_for_space_and_t_variants() {
        let mut request = base_request();
        request.from_date = Some("2025-06-09 13:15:00.123".to_string());
        let parsed_space = request
            .validate_from_date()
            .expect("space-separated fractional seconds should parse")
            .expect("from_date should be present");
        assert_eq!(
            parsed_space,
            DateTime::parse_from_rfc3339("2025-06-09T13:15:00.123Z")
                .expect("expected timestamp should parse")
                .with_timezone(&Utc)
        );

        request.from_date = Some("2025-06-09T13:15:00.456".to_string());
        let parsed_t = request
            .validate_from_date()
            .expect("T-separated fractional seconds should parse")
            .expect("from_date should be present");
        assert_eq!(
            parsed_t,
            DateTime::parse_from_rfc3339("2025-06-09T13:15:00.456Z")
                .expect("expected timestamp should parse")
                .with_timezone(&Utc)
        );
    }

    #[test]
    fn validate_from_date_rejects_invalid_formats() {
        let mut request = base_request();
        request.from_date = Some("2025-06-09".to_string());
        assert!(request.validate_from_date().is_err());

        request.from_date = Some("not-a-date".to_string());
        assert!(request.validate_from_date().is_err());
    }
}
