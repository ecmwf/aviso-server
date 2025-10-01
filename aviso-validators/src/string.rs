//! String validation and canonicalization
//!
//! Provides validation for string fields with configurable length constraints.
//! Ensures strings are non-empty and within specified length limits.

use anyhow::{Result, bail};

/// String validation handler
///
/// Validates string fields according to the following rules:
/// - Must not be empty (required for all string fields)
/// - Must not exceed maximum length if specified
/// - Returns the original string if valid (no canonicalization needed)
pub struct StringHandler;

impl StringHandler {
    /// Validate and canonicalize a string value
    ///
    /// # Arguments
    /// * `value` - The string value to validate
    /// * `max_length` - Optional maximum length constraint
    /// * `field_name` - Name of the field (for error messages)
    ///
    /// # Returns
    /// * `Ok(String)` - The validated string (unchanged)
    /// * `Err(anyhow::Error)` - Validation failed with detailed error
    pub fn validate_and_canonicalize(
        value: &str,
        max_length: Option<usize>,
        field_name: &str,
    ) -> Result<String> {
        // Check for empty strings (not allowed)
        if value.is_empty() {
            bail!("Field '{}' cannot be empty", field_name);
        }

        // Check maximum length constraint if specified
        if let Some(max_len) = max_length
            && value.len() > max_len
        {
            bail!(
                "Field '{}' exceeds maximum length of {} characters, got: {}",
                field_name,
                max_len,
                value.len()
            );
        }

        // String is valid, return as-is (no canonicalization needed)
        Ok(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_string() {
        let result = StringHandler::validate_and_canonicalize("test", Some(10), "field");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "test");
    }

    #[test]
    fn test_empty_string_fails() {
        let result = StringHandler::validate_and_canonicalize("", Some(10), "field");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("cannot be empty"));
    }

    #[test]
    fn test_too_long_string_fails() {
        let result = StringHandler::validate_and_canonicalize("toolong", Some(5), "field");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("exceeds maximum length")
        );
    }

    #[test]
    fn test_no_length_limit() {
        let result = StringHandler::validate_and_canonicalize("very long string", None, "field");
        assert!(result.is_ok());
    }
}
