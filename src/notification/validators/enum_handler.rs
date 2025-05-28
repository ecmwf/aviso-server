//! Enumeration validation and canonicalization handler
//!
//! Validates that field values match one of a predefined set of allowed values.
//! Performs case-insensitive matching and canonicalizes to lowercase for
//! consistent topic generation and storage.

use anyhow::{Result, bail};

pub struct EnumHandler;

impl EnumHandler {
    /// Validate and canonicalize an enumeration value
    ///
    /// This method performs case-insensitive validation against the allowed
    /// values list and canonicalizes the result to lowercase for consistency.
    ///
    /// # Validation Process
    /// - Convert input to lowercase for comparison
    /// - Check if lowercase value exists in allowed values (case-insensitive)
    /// - Return the lowercase canonical form if valid
    /// - Provide detailed error with all allowed values if invalid
    ///
    /// # Arguments
    /// * `value` - The input value to validate
    /// * `allowed_values` - List of allowed values (case-insensitive matching)
    /// * `field_name` - Name of the field being validated (for error messages)
    ///
    /// # Returns
    /// * `Ok(String)` - The value in canonical lowercase form
    /// * `Err(anyhow::Error)` - Value not in allowed list with helpful error
    pub fn validate_and_canonicalize(
        value: &str,
        allowed_values: &[String],
        field_name: &str,
    ) -> Result<String> {
        // Convert input to lowercase for case-insensitive comparison
        let lowercase_value = value.to_lowercase();

        // Check if the lowercase value matches any allowed value (case-insensitive)
        let is_valid = allowed_values
            .iter()
            .any(|allowed| allowed.to_lowercase() == lowercase_value);

        if is_valid {
            tracing::debug!(
                field_name = field_name,
                input_value = value,
                canonical_value = %lowercase_value,
                allowed_count = allowed_values.len(),
                "Enum value successfully validated and canonicalized"
            );

            Ok(lowercase_value)
        } else {
            // Provide helpful error message with all allowed values
            let allowed_display = if allowed_values.len() <= 10 {
                // Show all values if the list is short
                format!("[{}]", allowed_values.join(", "))
            } else {
                // Show first few values if the list is long
                format!(
                    "[{}, ... and {} more]",
                    allowed_values
                        .iter()
                        .take(5)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", "),
                    allowed_values.len() - 5
                )
            };

            bail!(
                "Field '{}' has invalid value '{}'. Allowed values: {}",
                field_name,
                value,
                allowed_display
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_enum_value_exact_case() {
        let allowed = vec![
            "active".to_string(),
            "inactive".to_string(),
            "pending".to_string(),
        ];
        let result = EnumHandler::validate_and_canonicalize("active", &allowed, "status");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "active");
    }

    #[test]
    fn test_valid_enum_value_different_case() {
        let allowed = vec!["active".to_string(), "inactive".to_string()];
        let result = EnumHandler::validate_and_canonicalize("ACTIVE", &allowed, "status");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "active");
    }

    #[test]
    fn test_valid_enum_value_mixed_case() {
        let allowed = vec!["Active".to_string(), "InActive".to_string()];
        let result = EnumHandler::validate_and_canonicalize("active", &allowed, "status");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "active");
    }

    #[test]
    fn test_invalid_enum_value() {
        let allowed = vec!["active".to_string(), "inactive".to_string()];
        let result = EnumHandler::validate_and_canonicalize("unknown", &allowed, "status");
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_allowed_values() {
        let allowed = vec![];
        let result = EnumHandler::validate_and_canonicalize("any", &allowed, "field");
        assert!(result.is_err());
    }

    #[test]
    fn test_large_allowed_values_list() {
        let allowed: Vec<String> = (0..20).map(|i| format!("value{}", i)).collect();
        let result = EnumHandler::validate_and_canonicalize("unknown", &allowed, "field");
        assert!(result.is_err());
    }
}
