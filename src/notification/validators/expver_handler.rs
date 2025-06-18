use anyhow::{Result, bail};

/// Experiment version validation handler
///
/// Handles experiment version identifiers for different runs and model versions
///
/// - **Numeric versions**: Integers that are zero-padded to 4 digits (e.g., 1 → "0001")
/// - **String versions**: Alphanumeric identifiers converted to lowercase (e.g., "PROD" → "prod")
///
/// The canonicalization ensures consistent representation for topic generation
/// and database storage while supporting the flexibility needed by different
/// operational workflows.
pub struct ExpverHandler;

impl ExpverHandler {
    /// Validate and canonicalize an experiment version value
    ///
    /// This method handles both numeric and string experiment versions:
    /// - Numeric values are zero-padded to 4 digits for consistency
    /// - String values are converted to lowercase for standardization
    /// - Empty values use the configured default if available
    ///
    /// # Arguments
    /// * `value` - The experiment version to validate (can be empty if default provided)
    /// * `default` - Optional default value to use when input is empty
    /// * `field_name` - Name of the field being validated (for error messages)
    ///
    /// # Returns
    /// * `Ok(String)` - The canonicalized experiment version
    /// * `Err(anyhow::Error)` - Empty value with no default provided
    pub fn validate_and_canonicalize(
        value: &str,
        default: Option<&str>,
        field_name: &str,
    ) -> Result<String> {
        // Handle empty values by using default if available
        if value.is_empty() {
            if let Some(default_val) = default {
                let canonicalized_default = Self::canonicalize_expver(default_val);
                tracing::debug!(
                    field_name = field_name,
                    default_value = default_val,
                    canonical_value = %canonicalized_default,
                    "Using default experiment version"
                );
                return Ok(canonicalized_default);
            } else {
                bail!("Field '{}' cannot be empty", field_name);
            }
        }

        // Canonicalize the provided value
        let canonicalized = Self::canonicalize_expver(value);

        tracing::debug!(
            field_name = field_name,
            input_value = value,
            canonical_value = %canonicalized,
            value_type = if value.parse::<u32>().is_ok() { "numeric" } else { "string" },
            "Experiment version successfully validated and canonicalized"
        );

        Ok(canonicalized)
    }

    /// Canonicalize an experiment version to standard format
    ///
    /// This method determines whether the input is numeric or string and
    /// applies the appropriate canonicalization rules:
    ///
    /// - **Numeric**: Zero-pad to 4 digits (supports up to 9999)
    /// - **String**: Convert to lowercase for consistency
    ///
    /// # Arguments
    /// * `value` - The experiment version value to canonicalize
    ///
    /// # Returns
    /// * `String` - The canonicalized experiment version
    ///
    /// # Numeric Canonicalization
    /// Numeric experiment versions are zero-padded to exactly 4 digits:
    /// - 1 → "0001"
    /// - 42 → "0042"  
    /// - 123 → "0123"
    /// - 9999 → "9999"
    ///
    /// # String Canonicalization
    /// String experiment versions are converted to lowercase:
    /// - "PROD" → "prod"
    /// - "Test" → "test"
    /// - "dev-branch" → "dev-branch"
    fn canonicalize_expver(value: &str) -> String {
        // Try to parse as integer first for numeric canonicalization
        if let Ok(num) = value.parse::<u32>() {
            // Zero-pad numeric values to 4 digits
            format!("{:04}", num)
        } else {
            // Convert string values to lowercase for consistency
            value.to_lowercase()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_numeric_expver_single_digit() {
        let result = ExpverHandler::validate_and_canonicalize("1", None, "expver");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "0001");
    }

    #[test]
    fn test_numeric_expver_multiple_digits() {
        let result = ExpverHandler::validate_and_canonicalize("42", None, "expver");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "0042");
    }

    #[test]
    fn test_numeric_expver_four_digits() {
        let result = ExpverHandler::validate_and_canonicalize("9999", None, "expver");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "9999");
    }

    #[test]
    fn test_string_expver_uppercase() {
        let result = ExpverHandler::validate_and_canonicalize("PROD", None, "expver");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "prod");
    }

    #[test]
    fn test_string_expver_mixed_case() {
        let result = ExpverHandler::validate_and_canonicalize("Test", None, "expver");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "test");
    }

    #[test]
    fn test_empty_with_numeric_default() {
        let result = ExpverHandler::validate_and_canonicalize("", Some("42"), "expver");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "0042");
    }

    #[test]
    fn test_empty_with_string_default() {
        let result = ExpverHandler::validate_and_canonicalize("", Some("PROD"), "expver");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "prod");
    }

    #[test]
    fn test_empty_without_default() {
        let result = ExpverHandler::validate_and_canonicalize("", None, "expver");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("cannot be empty"));
    }

    #[test]
    fn test_alphanumeric_string() {
        let result = ExpverHandler::validate_and_canonicalize("dev-v2.1", None, "expver");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "dev-v2.1");
    }
}
