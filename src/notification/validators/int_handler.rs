//! Integer validation and canonicalization handler
//!
//! Validates that field values are valid integers and optionally within
//! specified numeric ranges. Used for step values, counts, indices,
//! and other numeric parameters in operational systems.

use anyhow::{Context, Result, bail};

/// Integer validation handler
///
/// Validates and canonicalizes integer values with optional range constraints.
pub struct IntHandler;

impl IntHandler {
    /// Validate and canonicalize an integer value with optional range checking
    ///
    /// This method performs comprehensive integer validation:
    /// 1. Parses the input string as a signed 64-bit integer
    /// 2. Validates the value is within the specified range (if configured)
    /// 3. Returns the canonical string representation of the integer
    ///
    /// # Arguments
    /// * `value` - The string value to validate as an integer
    /// * `range` - Optional [min, max] range constraint (inclusive bounds)
    /// * `field_name` - Name of the field being validated (for error messages)
    ///
    /// # Returns
    /// * `Ok(String)` - The canonical string representation of the valid integer
    /// * `Err(anyhow::Error)` - Invalid integer format or out of range
    ///
    /// # Range Validation
    /// When a range is specified as `[min, max]`, both bounds are inclusive:
    /// - `[0, 100]` allows values from 0 to 100 (including 0 and 100)
    /// - `[1, 10]` allows values from 1 to 10 (including 1 and 10)
    /// - No range means any valid integer is accepted
    pub fn validate_and_canonicalize(
        value: &str,
        range: Option<&[i64; 2]>,
        field_name: &str,
    ) -> Result<String> {
        // Parse the input string as a signed 64-bit integer
        let parsed_value: i64 = value.parse().context(format!(
            "Field '{}' must be a valid integer, got: '{}'",
            field_name, value
        ))?;

        // Validate range constraints if specified
        if let Some([min, max]) = range {
            if parsed_value < *min || parsed_value > *max {
                bail!(
                    "Field '{}' value {} is outside allowed range [{}, {}]",
                    field_name,
                    parsed_value,
                    min,
                    max
                );
            }

            tracing::debug!(
                field_name = field_name,
                input_value = value,
                parsed_value = parsed_value,
                min_allowed = min,
                max_allowed = max,
                "Integer successfully validated within range"
            );
        } else {
            tracing::debug!(
                field_name = field_name,
                input_value = value,
                parsed_value = parsed_value,
                "Integer successfully validated (no range constraint)"
            );
        }

        // Return the canonical string representation
        // This ensures consistent formatting (removes leading zeros, etc.)
        Ok(parsed_value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_positive_integer() {
        let result = IntHandler::validate_and_canonicalize("42", None, "count");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "42");
    }

    #[test]
    fn test_valid_negative_integer() {
        let result = IntHandler::validate_and_canonicalize("-42", None, "offset");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "-42");
    }

    #[test]
    fn test_valid_zero() {
        let result = IntHandler::validate_and_canonicalize("0", Some(&[-10, 10]), "value");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "0");
    }

    #[test]
    fn test_valid_integer_within_range() {
        let result = IntHandler::validate_and_canonicalize("50", Some(&[0, 100]), "step");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "50");
    }

    #[test]
    fn test_valid_integer_at_range_boundaries() {
        // Test minimum boundary
        let result = IntHandler::validate_and_canonicalize("0", Some(&[0, 100]), "step");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "0");

        // Test maximum boundary
        let result = IntHandler::validate_and_canonicalize("100", Some(&[0, 100]), "step");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "100");
    }

    #[test]
    fn test_integer_below_minimum() {
        let result = IntHandler::validate_and_canonicalize("-5", Some(&[0, 100]), "step");
        assert!(result.is_err());
    }

    #[test]
    fn test_integer_above_maximum() {
        let result = IntHandler::validate_and_canonicalize("150", Some(&[0, 100]), "step");
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_integer_format() {
        let result = IntHandler::validate_and_canonicalize("abc", None, "count");
        assert!(result.is_err());
    }

    #[test]
    fn test_decimal_number_rejected() {
        let result = IntHandler::validate_and_canonicalize("42.5", None, "count");
        assert!(result.is_err());
    }

    #[test]
    fn test_leading_zeros_canonicalized() {
        let result = IntHandler::validate_and_canonicalize("0042", None, "count");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "42"); // Leading zeros removed
    }

    #[test]
    fn test_large_integer() {
        let result = IntHandler::validate_and_canonicalize("9223372036854775807", None, "big");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "9223372036854775807");
    }
}
