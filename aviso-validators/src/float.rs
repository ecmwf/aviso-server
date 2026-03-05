//! Floating-point validation and canonicalization handler.

use anyhow::{Context, Result, bail};

/// Floating-point validation handler.
pub struct FloatHandler;

impl FloatHandler {
    /// Validates a floating-point field, applies optional inclusive range checks,
    /// and returns a canonical string representation.
    ///
    /// Valid example: `"12.5"` -> `"12.5"`.
    /// Invalid example: `"NaN"` -> error.
    ///
    /// Canonicalization uses Rust's shortest round-trippable decimal formatting.
    /// Non-finite values (`NaN`, `inf`, `-inf`) are rejected.
    pub fn validate_and_canonicalize(
        value: &str,
        range: Option<&[f64; 2]>,
        field_name: &str,
    ) -> Result<String> {
        let parsed_value: f64 = value.parse().context(format!(
            "Field '{}' must be a valid number, got: '{}'",
            field_name, value
        ))?;

        if !parsed_value.is_finite() {
            bail!(
                "Field '{}' must be a finite number, got: '{}'",
                field_name,
                value
            );
        }

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
        }

        Ok(parsed_value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::FloatHandler;

    #[test]
    fn valid_float_without_range() {
        let result = FloatHandler::validate_and_canonicalize("12.5", None, "severity");
        assert!(result.is_ok());
        assert_eq!(result.expect("value should be valid"), "12.5");
    }

    #[test]
    fn valid_float_with_range() {
        let result = FloatHandler::validate_and_canonicalize("3.4", Some(&[1.0, 7.0]), "level");
        assert!(result.is_ok());
    }

    #[test]
    fn invalid_float_format() {
        let result = FloatHandler::validate_and_canonicalize("abc", None, "level");
        assert!(result.is_err());
    }

    #[test]
    fn float_outside_range() {
        let result = FloatHandler::validate_and_canonicalize("9.1", Some(&[1.0, 7.0]), "level");
        assert!(result.is_err());
    }

    #[test]
    fn rejects_nan() {
        let result = FloatHandler::validate_and_canonicalize("NaN", None, "level");
        assert!(result.is_err());
    }

    #[test]
    fn rejects_positive_infinity() {
        let result = FloatHandler::validate_and_canonicalize("inf", None, "level");
        assert!(result.is_err());
    }

    #[test]
    fn rejects_negative_infinity() {
        let result = FloatHandler::validate_and_canonicalize("-inf", None, "level");
        assert!(result.is_err());
    }
}
