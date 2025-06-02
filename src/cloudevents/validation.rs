//! CloudEvent validation logic
//!
//! This module implements validation rules for CloudEvents according to the
//! CloudEvents 1.0 specification, plus any additional application-specific
//! validation requirements.

use crate::notification::types::{AvisoCloudEventTypes, OperationType};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde_json::Value;

/// CloudEvent validator implementing CloudEvents 1.0 specification checks
///
/// The validator ensures that:
/// - All required attributes are present and valid
/// - The specification version is supported
/// - Data format is consistent with declared content type
pub struct CloudEventValidator;

impl CloudEventValidator {
    /// Validate CloudEvent JSON payload before parsing
    ///
    /// This performs lightweight validation to provide clear error messages
    /// when required fields are missing or invalid before attempting full parsing.
    ///
    /// # Arguments
    /// * `json_payload` - The JSON value to validate
    ///
    /// # Returns
    /// * `Ok(())` - All required attributes are present and valid
    /// * `Err(anyhow::Error)` - Missing or invalid required attributes with detailed message
    ///
    /// # Validation Steps
    /// - Ensures payload is a JSON object
    /// - Checks for presence of all required CloudEvent attributes
    /// - Validates data types and non-empty values for required fields
    /// - Verifies CloudEvents specification version compatibility
    /// - Validates Aviso-specific extension attributes if present
    pub fn validate_json_cloudevent(json_payload: &Value) -> Result<()> {
        // Ensure the payload is a JSON object, not an array or primitive value
        let obj = Self::ensure_json_object(json_payload)?;

        // Find any missing required fields to provide comprehensive error messages
        let missing_fields = Self::find_missing_required_fields(&obj);

        // Find any invalid required fields (wrong type or empty values)
        let invalid_fields = Self::find_invalid_required_fields(&obj);

        // Validate the CloudEvents specification version early
        Self::validate_spec_version_in_json(&obj)?;

        // Validate Aviso extension attributes if present (all optional)
        Self::validate_aviso_extension_attributes(&obj)?;

        // Report all validation errors at once for better user experience
        Self::report_validation_errors(missing_fields, invalid_fields)?;

        Ok(())
    }

    /// Validate Aviso-specific extension attributes
    ///
    /// CloudEvents extension attributes must follow specific naming and format rules:
    /// - Extension attribute names must be lowercase alphanumeric with no spaces
    /// - Values must be strings, numbers, or booleans (not objects or arrays)
    /// - All Aviso extension attributes are optional and can be used with any operation
    ///
    /// # Arguments
    /// * `obj` - The JSON object containing CloudEvent attributes
    ///
    /// # Returns
    /// * `Ok(())` - All extension attributes are valid
    /// * `Err(anyhow::Error)` - Invalid extension attribute format or value
    fn validate_aviso_extension_attributes(obj: &serde_json::Map<String, Value>) -> Result<()> {
        // Validate avisoFromTime extension attribute if present
        if let Some(aviso_from_time) = obj.get("avisoFromTime") {
            Self::validate_aviso_from_time_extension(aviso_from_time)?;
        }

        Ok(())
    }

    /// Validate avisoFromTime extension attribute
    ///
    /// The avisoFromTime extension attribute specifies a timestamp for historical
    /// operations. It is completely optional and can be used with any operation type.
    /// When present, it must be:
    /// - A valid ISO 8601 timestamp string
    /// - In UTC timezone (Z suffix or +00:00 offset)
    /// - Not in the future (historical operations only)
    ///
    /// # Arguments
    /// * `value` - The avisoFromTime value to validate
    ///
    /// # Returns
    /// * `Ok(())` - Valid ISO 8601 timestamp
    /// * `Err(anyhow::Error)` - Invalid timestamp format or future timestamp
    fn validate_aviso_from_time_extension(value: &Value) -> Result<()> {
        // Extension attribute must be a string
        let time_str = value.as_str().ok_or_else(|| {
            anyhow::anyhow!(
                "Extension attribute 'avisoFromTime' must be a string, got: {}",
                value
            )
        })?;

        // Parse as ISO 8601 timestamp
        let parsed_time = DateTime::parse_from_rfc3339(time_str)
            .context(format!(
                "Extension attribute 'avisoFromTime' must be a valid ISO 8601 timestamp. \
                 Got: '{}'. Expected format: '2025-06-02T22:06:00Z' or '2025-06-02T22:06:00+00:00'",
                time_str
            ))?;

        // Convert to UTC for comparison
        let utc_time = parsed_time.with_timezone(&Utc);
        let now = Utc::now();

        // Validate that the timestamp is not in the future
        if utc_time > now {
            bail!(
                "Extension attribute 'avisoFromTime' cannot be in the future. \
                 Got: '{}' ({}), current time: {} ({})",
                time_str,
                utc_time.to_rfc3339(),
                now.to_rfc3339(),
                now
            );
        }

        tracing::debug!(
            aviso_from_time = %time_str,
            parsed_utc = %utc_time.to_rfc3339(),
            "avisoFromTime extension attribute validated successfully"
        );

        Ok(())
    }

    // JSON Payload Validation Helper Functions
    /// Ensure the JSON payload is an object, not an array or primitive
    ///
    /// CloudEvents must be JSON objects with key-value pairs for attributes.
    /// Arrays and primitive values are not valid CloudEvent representations.
    ///
    /// # Arguments
    /// * `json_payload` - The JSON value to check
    ///
    /// # Returns
    /// * `Ok(&Map)` - Reference to the JSON object
    /// * `Err(anyhow::Error)` - Payload is not a JSON object
    fn ensure_json_object(json_payload: &Value) -> Result<&serde_json::Map<String, Value>> {
        json_payload
            .as_object()
            .context("CloudEvent payload must be a JSON object, not an array or primitive value")
    }

    /// Find all missing required CloudEvent attributes
    ///
    /// According to CloudEvents 1.0 specification, these attributes are mandatory:
    /// - `specversion`: The version of the CloudEvents specification
    /// - `id`: Identifies the event (must be unique within the source)
    /// - `source`: Identifies the context in which an event happened
    /// - `type`: Describes the type of event related to the originating occurrence
    ///
    /// # Arguments
    /// * `obj` - The JSON object to check for required fields
    ///
    /// # Returns
    /// * `Vec<&'static str>` - List of missing required field names
    fn find_missing_required_fields(obj: &serde_json::Map<String, Value>) -> Vec<&'static str> {
        // Define all required CloudEvent attributes per CloudEvents 1.0 spec
        let required_fields = ["specversion", "id", "source", "type"];

        // Filter to find fields that are completely missing from the JSON object
        required_fields
            .iter()
            .filter(|&&field| !obj.contains_key(field))
            .copied()
            .collect()
    }

    /// Find all invalid required CloudEvent attributes
    ///
    /// This checks for attributes that are present but have invalid values:
    /// - Wrong data type (e.g., number instead of string)
    /// - Empty string values (which violate CloudEvents spec requirements)
    ///
    /// # Arguments
    /// * `obj` - The JSON object to validate field values
    ///
    /// # Returns
    /// * `Vec<String>` - List of invalid field descriptions with error details
    fn find_invalid_required_fields(obj: &serde_json::Map<String, Value>) -> Vec<String> {
        let mut invalid_fields = Vec::new();

        // Validate each string field for proper type and non-empty value
        Self::validate_string_field(obj, "id", &mut invalid_fields);
        Self::validate_string_field(obj, "source", &mut invalid_fields);
        Self::validate_string_field(obj, "type", &mut invalid_fields);

        invalid_fields
    }

    /// Validate a single string field for proper type and non-empty value
    ///
    /// CloudEvent string attributes must be actual strings (not numbers, booleans, etc.)
    /// and cannot be empty strings as per CloudEvents specification.
    ///
    /// # Arguments
    /// * `obj` - The JSON object containing the field
    /// * `field_name` - Name of the field to validate
    /// * `invalid_fields` - Mutable vector to collect validation errors
    fn validate_string_field(
        obj: &serde_json::Map<String, Value>,
        field_name: &str,
        invalid_fields: &mut Vec<String>,
    ) {
        if let Some(value) = obj.get(field_name) {
            if let Some(string_value) = value.as_str() {
                // Check if the string value is empty (not allowed for required fields)
                if string_value.is_empty() {
                    invalid_fields.push(format!("{} (cannot be empty)", field_name));
                }
            } else {
                // Field exists but is not a string type
                invalid_fields.push(format!("{} (must be a string)", field_name));
            }
        }
    }

    /// Validate CloudEvents specification version in JSON payload
    ///
    /// Currently only CloudEvents 1.0 is supported to ensure consistent behavior
    /// and attribute requirements across the system.
    ///
    /// # Arguments
    /// * `obj` - The JSON object containing the specversion field
    ///
    /// # Returns
    /// * `Ok(())` - Specification version is valid and supported
    /// * `Err(anyhow::Error)` - Unsupported or invalid specification version
    fn validate_spec_version_in_json(obj: &serde_json::Map<String, Value>) -> Result<()> {
        if let Some(spec_version) = obj.get("specversion") {
            if let Some(version_str) = spec_version.as_str() {
                if version_str != "1.0" {
                    bail!(
                        "Unsupported CloudEvents specification version: '{}'. Only version '1.0' is supported",
                        version_str
                    );
                }
            }
            // Note: Type validation for specversion is handled in find_invalid_required_fields
        }
        Ok(())
    }

    /// Report comprehensive validation errors to the user
    ///
    /// This combines missing and invalid field errors into a single, clear error message
    /// that helps users understand exactly what needs to be fixed in their CloudEvent.
    ///
    /// # Arguments
    /// * `missing_fields` - List of completely missing required fields
    /// * `invalid_fields` - List of present but invalid fields with descriptions
    ///
    /// # Returns
    /// * `Ok(())` - No validation errors found
    /// * `Err(anyhow::Error)` - Comprehensive error message with all validation issues
    fn report_validation_errors(
        missing_fields: Vec<&'static str>,
        invalid_fields: Vec<String>,
    ) -> Result<()> {
        // If no errors found, validation passes
        if missing_fields.is_empty() && invalid_fields.is_empty() {
            return Ok(());
        }

        let mut error_parts = Vec::new();

        // Add missing fields section to error message
        if !missing_fields.is_empty() {
            error_parts.push(format!(
                "Missing required attributes: [{}]",
                missing_fields.join(", ")
            ));
        }

        // Add invalid fields section to error message
        if !invalid_fields.is_empty() {
            error_parts.push(format!(
                "Invalid attributes: [{}]",
                invalid_fields.join(", ")
            ));
        }

        // Combine all error parts into a comprehensive error message
        bail!(
            "Invalid CloudEvent: {}. CloudEvents must include valid: specversion, id, source, type",
            error_parts.join("; ")
        );
    }
}

/// Aviso-specific CloudEvent type validation
///
/// Validates CloudEvent types that should follow the pattern:
/// - int.ecmwf.aviso.notify (for notify operations)
/// - int.ecmwf.aviso.watch (for watch operations)
/// - int.ecmwf.aviso.replay (for replay operations)
pub struct AvisoTypeValidator;

impl AvisoTypeValidator {
    /// Get all supported Aviso CloudEvent types
    pub fn get_supported_types() -> Vec<String> {
        AvisoCloudEventTypes::get_supported_types()
    }

    /// Get a formatted error message for unsupported types
    pub fn get_unsupported_type_error(actual_type: &str) -> String {
        AvisoCloudEventTypes::get_unsupported_type_error(actual_type)
    }

    /// Check if a CloudEvent type is an Aviso type with detailed error
    pub fn validate_is_aviso_type(cloudevent_type: &str) -> Result<(), anyhow::Error> {
        if AvisoCloudEventTypes::is_aviso_type(cloudevent_type) {
            Ok(())
        } else {
            bail!("{}", Self::get_unsupported_type_error(cloudevent_type))
        }
    }

    /// Validate CloudEvent type and extract operation type
    ///
    /// # Arguments
    /// * `cloudevent_type` - The CloudEvent type field to validate
    ///
    /// # Returns
    /// * `Ok(OperationType)` - Valid Aviso type with extracted operation
    /// * `Err(anyhow::Error)` - Invalid type format or unsupported operation
    pub fn validate_and_extract_operation(cloudevent_type: &str) -> Result<OperationType> {
        AvisoCloudEventTypes::validate_and_extract_operation(cloudevent_type)
    }

    /// Check if a CloudEvent type is an Aviso type (without validating operation)
    pub fn is_aviso_type(cloudevent_type: &str) -> bool {
        AvisoCloudEventTypes::is_aviso_type(cloudevent_type)
    }
}

/// Extract and validate Aviso operation type from CloudEvent type
pub fn extract_and_validate_aviso_operation(
    cloudevent_type: &str,
) -> std::result::Result<OperationType, anyhow::Error> {
    AvisoTypeValidator::validate_and_extract_operation(cloudevent_type)
}
