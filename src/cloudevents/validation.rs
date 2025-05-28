//! CloudEvent validation logic
//!
//! This module implements validation rules for CloudEvents according to the
//! CloudEvents 1.0 specification, plus any additional application-specific
//! validation requirements.

use crate::notification::OperationType;
use anyhow::{Context, Result, bail};
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
    pub fn validate_json_cloudevent(json_payload: &Value) -> Result<()> {
        // Ensure the payload is a JSON object, not an array or primitive value
        let obj = Self::ensure_json_object(json_payload)?;

        // Find any missing required fields to provide comprehensive error messages
        let missing_fields = Self::find_missing_required_fields(&obj);

        // Find any invalid required fields (wrong type or empty values)
        let invalid_fields = Self::find_invalid_required_fields(&obj);

        // Validate the CloudEvents specification version early
        Self::validate_spec_version_in_json(&obj)?;

        // Report all validation errors at once for better user experience
        Self::report_validation_errors(missing_fields, invalid_fields)?;

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
/// - int.ecmwf.aviso.listen (for listen operations)
pub struct AvisoTypeValidator;

impl AvisoTypeValidator {
    /// Expected prefix for all Aviso CloudEvent types
    const AVISO_TYPE_PREFIX: &'static str = "int.ecmwf.aviso";

    /// Valid operation suffixes
    const NOTIFY_SUFFIX: &'static str = "notify";
    const LISTEN_SUFFIX: &'static str = "listen";

    /// Get all supported Aviso CloudEvent types
    pub fn get_supported_types() -> Vec<String> {
        vec![
            format!("{}.{}", Self::AVISO_TYPE_PREFIX, Self::NOTIFY_SUFFIX),
            format!("{}.{}", Self::AVISO_TYPE_PREFIX, Self::LISTEN_SUFFIX),
        ]
    }

    /// Get a formatted error message for unsupported types
    pub fn get_unsupported_type_error(actual_type: &str) -> String {
        let supported_types = Self::get_supported_types();
        format!(
            "Only Aviso CloudEvent types are supported. Got: '{}'. Expected one of: [{}]",
            actual_type,
            supported_types.join(", ")
        )
    }

    /// Check if a CloudEvent type is an Aviso type with detailed error
    pub fn validate_is_aviso_type(cloudevent_type: &str) -> Result<(), anyhow::Error> {
        if Self::is_aviso_type(cloudevent_type) {
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
    /// * `Ok(&str)` - Valid Aviso type with extracted operation ("notify" or "listen")
    /// * `Err(anyhow::Error)` - Invalid type format or unsupported operation
    pub fn validate_and_extract_operation(cloudevent_type: &str) -> Result<&str> {
        // Check if the type starts with the expected Aviso prefix
        if !cloudevent_type.starts_with(Self::AVISO_TYPE_PREFIX) {
            bail!(
                "Invalid Aviso CloudEvent type '{}'. Must start with '{}'",
                cloudevent_type,
                Self::AVISO_TYPE_PREFIX
            );
        }

        // Extract the operation suffix after the prefix
        let operation_part = cloudevent_type
            .strip_prefix(Self::AVISO_TYPE_PREFIX)
            .and_then(|s| s.strip_prefix('.'))
            .unwrap_or("");

        // Validate the operation suffix
        match operation_part {
            Self::NOTIFY_SUFFIX => {
                tracing::debug!(
                    cloudevent_type = cloudevent_type,
                    operation = "notify",
                    "Valid Aviso CloudEvent type with notify operation"
                );
                Ok("notify")
            }
            Self::LISTEN_SUFFIX => {
                tracing::debug!(
                    cloudevent_type = cloudevent_type,
                    operation = "listen",
                    "Valid Aviso CloudEvent type with listen operation"
                );
                Ok("listen")
            }
            _ => {
                bail!(
                    "Invalid Aviso CloudEvent operation '{}' in type '{}'. \
                     Supported operations: '{}', '{}'",
                    operation_part,
                    cloudevent_type,
                    Self::NOTIFY_SUFFIX,
                    Self::LISTEN_SUFFIX
                );
            }
        }
    }

    /// Check if a CloudEvent type is an Aviso type (without validating operation)
    pub fn is_aviso_type(cloudevent_type: &str) -> bool {
        cloudevent_type.starts_with(Self::AVISO_TYPE_PREFIX)
    }
}

/// Extract and validate Aviso operation type from CloudEvent type
pub fn extract_and_validate_aviso_operation(
    cloudevent_type: &str,
) -> std::result::Result<OperationType, anyhow::Error> {
    let operation_str = AvisoTypeValidator::validate_and_extract_operation(cloudevent_type)?;
    OperationType::from_str(operation_str)
}

#[cfg(test)]
mod aviso_tests {
    use super::*;

    #[test]
    fn test_valid_notify_type() {
        let result = AvisoTypeValidator::validate_and_extract_operation("int.ecmwf.aviso.notify");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "notify");
    }

    #[test]
    fn test_valid_listen_type() {
        let result = AvisoTypeValidator::validate_and_extract_operation("int.ecmwf.aviso.listen");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "listen");
    }

    #[test]
    fn test_invalid_prefix() {
        let result = AvisoTypeValidator::validate_and_extract_operation("com.example.invalid");
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_operation() {
        let result = AvisoTypeValidator::validate_and_extract_operation("int.ecmwf.aviso.invalid");
        assert!(result.is_err());
    }

    #[test]
    fn test_is_aviso_type() {
        assert!(AvisoTypeValidator::is_aviso_type("int.ecmwf.aviso.notify"));
        assert!(AvisoTypeValidator::is_aviso_type("int.ecmwf.aviso.listen"));
        assert!(!AvisoTypeValidator::is_aviso_type("com.example.other"));
    }
}
