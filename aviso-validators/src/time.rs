// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Time validation and canonicalization handler
//!
//! Handles validation and canonicalization of time values used in meteorological
//! and operational systems. Supports multiple input formats and converts them
//! to a consistent HHMM format for topic generation and storage.

use anyhow::{Context, Result, bail};
use regex::Regex;

/// Time validation handler
///
/// Validates and canonicalizes time values. Supports multiple input formats:
///
/// - **Single or double-digit hours**: 0, 1, 01, 23 (converted to HH00 format)
/// - **HH:MM format**: Standard time with colon separator (e.g., "14:30")
/// - **HHMM format**: Compact time without separator (e.g., "1430")
///
/// All valid inputs are canonicalized to HHMM format (4-digit string) for
/// consistent representation in topics and storage systems.
pub struct TimeHandler;

impl TimeHandler {
    /// Validate and canonicalize a time value to HHMM format
    ///
    /// This method handles multiple time input formats and converts them to
    /// the standard HHMM format used throughout the system:
    ///
    /// # Supported Input Formats
    /// 1. **Single or double-digit hours**: "0", "1", "01", "23" → "0000", "0100", "0100", "2300"
    /// 2. **HH:MM format**: "14:30" → "1430", "9:15" → "0915"
    /// 3. **HHMM format**: "1430" → "1430" (validated and normalized)
    ///
    /// # Arguments
    /// * `value` - The time string to validate (in any supported format)
    /// * `field_name` - Name of the field being validated (for error messages)
    ///
    /// # Returns
    /// * `Ok(String)` - The time in canonical HHMM format
    /// * `Err(anyhow::Error)` - Invalid time format or impossible time
    ///
    /// # Time Validation Rules
    /// - Hours must be 0-23 (24-hour format)
    /// - Minutes must be 0-59
    /// - Leading zeros are added as needed for consistent 4-digit format
    pub fn validate_and_canonicalize(value: &str, field_name: &str) -> Result<String> {
        // Handle single or double-digit hours (0, 01, 1, ..., 23)
        if let Some(canonical_time) = Self::handle_hour_only_format(value, field_name)? {
            tracing::debug!(
                field_name = field_name,
                input_value = value,
                canonical_value = %canonical_time,
                format_type = "hour only",
                "Time successfully validated and canonicalized"
            );
            return Ok(canonical_time);
        }

        // Handle HH:MM format (with colon separator)
        if value.contains(':') {
            return Self::handle_colon_format(value, field_name);
        }

        // Handle HHMM format (4-digit compact format)
        if value.len() == 4 {
            return Self::handle_compact_format(value, field_name);
        }

        // No supported format matched
        bail!(
            "Field '{}' has invalid time format '{}'. Expected: H, HH, HH:MM, or HHMM",
            field_name,
            value
        );
    }

    /// Handle hour-only values (0, 1, 01, 23, etc.)
    ///
    /// This method handles single digit and double-digit hour values without
    /// minutes, converting them to HHMM format with minutes set to 00.
    /// It treats all numeric values of 1-2 digits as hours.
    ///
    /// # Arguments
    /// * `value` - The input time value
    /// * `field_name` - Name of the field (for error messages)
    ///
    /// # Returns
    /// * `Ok(Some(String))` - Canonical time if input is a valid hour
    /// * `Ok(None)` - Input is not a 1-2 digit numeric value
    /// * `Err(anyhow::Error)` - Invalid hour value (out of range)
    ///
    /// # Examples
    /// - "0" → "0000"
    /// - "1" → "0100"
    /// - "01" → "0100"
    /// - "9" → "0900"
    /// - "23" → "2300"
    /// - "24" → Error (invalid hour)
    fn handle_hour_only_format(value: &str, field_name: &str) -> Result<Option<String>> {
        // Check if the value is 1-2 digits and all numeric
        if !value.is_empty() && value.len() <= 2 && value.chars().all(|c| c.is_ascii_digit()) {
            let hour: u32 = value.parse().context(format!(
                "Invalid hour value '{}' in field '{}'",
                value, field_name
            ))?;

            // Validate hour range (0-23)
            if hour > 23 {
                bail!(
                    "Field '{}' has invalid hours: {}. Hours must be 0-23 in 24-hour format",
                    field_name,
                    hour
                );
            }

            // Convert to HHMM format with minutes = 00
            let canonical_time = format!("{:02}00", hour);

            tracing::debug!(
                field_name = field_name,
                input_value = value,
                canonical_value = %canonical_time,
                parsed_hour = hour,
                "Hour-only time successfully parsed and canonicalized"
            );

            Ok(Some(canonical_time))
        } else {
            // Not a 1-2 digit numeric value
            Ok(None)
        }
    }

    /// Handle HH:MM format with colon separator
    ///
    /// Parses time values in the common HH:MM format and validates
    /// that hours and minutes are within valid ranges.
    ///
    /// # Arguments
    /// * `value` - The time string in HH:MM format
    /// * `field_name` - Name of the field (for error messages)
    ///
    /// # Returns
    /// * `Ok(String)` - Canonical HHMM format
    /// * `Err(anyhow::Error)` - Invalid format or out-of-range values
    fn handle_colon_format(value: &str, field_name: &str) -> Result<String> {
        let time_regex = Regex::new(r"^(\d{1,2}):(\d{2})$").unwrap();

        if let Some(captures) = time_regex.captures(value) {
            let hours: u32 = captures[1].parse().context(format!(
                "Invalid hours '{}' in field '{}'",
                &captures[1], field_name
            ))?;
            let minutes: u32 = captures[2].parse().context(format!(
                "Invalid minutes '{}' in field '{}'",
                &captures[2], field_name
            ))?;

            // Validate hour range (0-23)
            if hours > 23 {
                bail!(
                    "Field '{}' has invalid hours: {}. Hours must be 0-23 in 24-hour format",
                    field_name,
                    hours
                );
            }

            // Validate minute range (0-59)
            if minutes > 59 {
                bail!(
                    "Field '{}' has invalid minutes: {}. Minutes must be 0-59",
                    field_name,
                    minutes
                );
            }

            let canonical_time = format!("{:02}{:02}", hours, minutes);

            tracing::debug!(
                field_name = field_name,
                input_value = value,
                canonical_value = %canonical_time,
                format_type = "HH:MM",
                hours = hours,
                minutes = minutes,
                "Time successfully validated and canonicalized"
            );

            Ok(canonical_time)
        } else {
            bail!(
                "Field '{}' has invalid HH:MM format '{}'. Expected format: HH:MM (e.g., 14:30, 9:05)",
                field_name,
                value
            );
        }
    }

    /// Handle HHMM compact format (4 digits)
    ///
    /// Validates time values in the compact 4-digit HHMM format
    /// and ensures hours and minutes are within valid ranges.
    ///
    /// # Arguments
    /// * `value` - The time string in HHMM format
    /// * `field_name` - Name of the field (for error messages)
    ///
    /// # Returns
    /// * `Ok(String)` - Validated HHMM format (same as input if valid)
    /// * `Err(anyhow::Error)` - Invalid format or out-of-range values
    fn handle_compact_format(value: &str, field_name: &str) -> Result<String> {
        let time_regex = Regex::new(r"^(\d{2})(\d{2})$").unwrap();

        if let Some(captures) = time_regex.captures(value) {
            let hours: u32 = captures[1].parse().context(format!(
                "Invalid hours '{}' in field '{}'",
                &captures[1], field_name
            ))?;
            let minutes: u32 = captures[2].parse().context(format!(
                "Invalid minutes '{}' in field '{}'",
                &captures[2], field_name
            ))?;

            // Validate hour range (0-23)
            if hours > 23 {
                bail!(
                    "Field '{}' has invalid hours: {}. Hours must be 0-23 in 24-hour format",
                    field_name,
                    hours
                );
            }

            // Validate minute range (0-59)
            if minutes > 59 {
                bail!(
                    "Field '{}' has invalid minutes: {}. Minutes must be 0-59",
                    field_name,
                    minutes
                );
            }

            tracing::debug!(
                field_name = field_name,
                input_value = value,
                canonical_value = value,
                format_type = "HHMM",
                hours = hours,
                minutes = minutes,
                "Time successfully validated and canonicalized"
            );

            Ok(value.to_string())
        } else {
            bail!(
                "Field '{}' has invalid HHMM format '{}'. Expected format: HHMM with exactly 4 digits (e.g., 1430, 0905)",
                field_name,
                value
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test single digit hours (0-9)
    #[test]
    fn test_single_digit_hours() {
        assert_eq!(
            TimeHandler::validate_and_canonicalize("0", "time").unwrap(),
            "0000"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("1", "time").unwrap(),
            "0100"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("2", "time").unwrap(),
            "0200"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("3", "time").unwrap(),
            "0300"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("4", "time").unwrap(),
            "0400"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("5", "time").unwrap(),
            "0500"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("6", "time").unwrap(),
            "0600"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("7", "time").unwrap(),
            "0700"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("8", "time").unwrap(),
            "0800"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("9", "time").unwrap(),
            "0900"
        );
    }

    // Test double digit hours with leading zeros (00-09)
    #[test]
    fn test_double_digit_hours_with_leading_zeros() {
        assert_eq!(
            TimeHandler::validate_and_canonicalize("00", "time").unwrap(),
            "0000"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("01", "time").unwrap(),
            "0100"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("02", "time").unwrap(),
            "0200"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("03", "time").unwrap(),
            "0300"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("04", "time").unwrap(),
            "0400"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("05", "time").unwrap(),
            "0500"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("06", "time").unwrap(),
            "0600"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("07", "time").unwrap(),
            "0700"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("08", "time").unwrap(),
            "0800"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("09", "time").unwrap(),
            "0900"
        );
    }

    // Test double digit hours (10-23)
    #[test]
    fn test_double_digit_hours() {
        assert_eq!(
            TimeHandler::validate_and_canonicalize("10", "time").unwrap(),
            "1000"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("11", "time").unwrap(),
            "1100"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("12", "time").unwrap(),
            "1200"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("13", "time").unwrap(),
            "1300"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("14", "time").unwrap(),
            "1400"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("15", "time").unwrap(),
            "1500"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("16", "time").unwrap(),
            "1600"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("17", "time").unwrap(),
            "1700"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("18", "time").unwrap(),
            "1800"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("19", "time").unwrap(),
            "1900"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("20", "time").unwrap(),
            "2000"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("21", "time").unwrap(),
            "2100"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("22", "time").unwrap(),
            "2200"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("23", "time").unwrap(),
            "2300"
        );
    }

    // Test invalid hours
    #[test]
    fn test_invalid_hours() {
        assert!(TimeHandler::validate_and_canonicalize("24", "time").is_err());
        assert!(TimeHandler::validate_and_canonicalize("25", "time").is_err());
        assert!(TimeHandler::validate_and_canonicalize("99", "time").is_err());
    }

    // Test HH:MM format with single digit hours
    #[test]
    fn test_hh_mm_format_single_digit_hours() {
        assert_eq!(
            TimeHandler::validate_and_canonicalize("0:00", "time").unwrap(),
            "0000"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("1:15", "time").unwrap(),
            "0115"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("9:30", "time").unwrap(),
            "0930"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("9:05", "time").unwrap(),
            "0905"
        );
    }

    // Test HH:MM format with double digit hours
    #[test]
    fn test_hh_mm_format_double_digit_hours() {
        assert_eq!(
            TimeHandler::validate_and_canonicalize("10:00", "time").unwrap(),
            "1000"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("14:30", "time").unwrap(),
            "1430"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("23:59", "time").unwrap(),
            "2359"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("00:00", "time").unwrap(),
            "0000"
        );
    }

    // Test HH:MM format boundary cases
    #[test]
    fn test_hh_mm_format_boundaries() {
        assert_eq!(
            TimeHandler::validate_and_canonicalize("00:00", "time").unwrap(),
            "0000"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("23:59", "time").unwrap(),
            "2359"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("12:00", "time").unwrap(),
            "1200"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("0:59", "time").unwrap(),
            "0059"
        );
    }

    // Test invalid HH:MM format
    #[test]
    fn test_invalid_hh_mm_format() {
        // Invalid hours
        assert!(TimeHandler::validate_and_canonicalize("24:00", "time").is_err());
        assert!(TimeHandler::validate_and_canonicalize("25:30", "time").is_err());

        // Invalid minutes
        assert!(TimeHandler::validate_and_canonicalize("12:60", "time").is_err());
        assert!(TimeHandler::validate_and_canonicalize("12:99", "time").is_err());

        // Invalid format
        assert!(TimeHandler::validate_and_canonicalize("12:5", "time").is_err()); // Single digit minutes
        assert!(TimeHandler::validate_and_canonicalize("1:2", "time").is_err()); // Single digit minutes
    }

    // Test HHMM format
    #[test]
    fn test_hhmm_format() {
        assert_eq!(
            TimeHandler::validate_and_canonicalize("0000", "time").unwrap(),
            "0000"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("0100", "time").unwrap(),
            "0100"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("0905", "time").unwrap(),
            "0905"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("1430", "time").unwrap(),
            "1430"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("2359", "time").unwrap(),
            "2359"
        );
    }

    // Test invalid HHMM format
    #[test]
    fn test_invalid_hhmm_format() {
        // Invalid hours
        assert!(TimeHandler::validate_and_canonicalize("2400", "time").is_err());
        assert!(TimeHandler::validate_and_canonicalize("2500", "time").is_err());

        // Invalid minutes
        assert!(TimeHandler::validate_and_canonicalize("1260", "time").is_err());
        assert!(TimeHandler::validate_and_canonicalize("1299", "time").is_err());

        // Wrong length
        assert!(TimeHandler::validate_and_canonicalize("123", "time").is_err()); // Too short
        assert!(TimeHandler::validate_and_canonicalize("12345", "time").is_err()); // Too long
    }

    // Test completely invalid formats
    #[test]
    fn test_invalid_formats() {
        assert!(TimeHandler::validate_and_canonicalize("", "time").is_err());
        assert!(TimeHandler::validate_and_canonicalize("invalid", "time").is_err());
        assert!(TimeHandler::validate_and_canonicalize("abc", "time").is_err());
        assert!(TimeHandler::validate_and_canonicalize("12:ab", "time").is_err());
        assert!(TimeHandler::validate_and_canonicalize("ab:30", "time").is_err());
        assert!(TimeHandler::validate_and_canonicalize("12.30", "time").is_err()); // Wrong separator
        assert!(TimeHandler::validate_and_canonicalize("12-30", "time").is_err()); // Wrong separator
    }

    // Test edge cases with spaces and special characters
    #[test]
    fn test_edge_cases() {
        // Spaces should fail
        assert!(TimeHandler::validate_and_canonicalize(" 12", "time").is_err());
        assert!(TimeHandler::validate_and_canonicalize("12 ", "time").is_err());
        assert!(TimeHandler::validate_and_canonicalize(" 12:30", "time").is_err());
        assert!(TimeHandler::validate_and_canonicalize("12:30 ", "time").is_err());

        // Mixed characters should fail
        assert!(TimeHandler::validate_and_canonicalize("1a", "time").is_err());
        assert!(TimeHandler::validate_and_canonicalize("a1", "time").is_err());
    }

    // Test consistency between different input formats for same time
    #[test]
    fn test_format_consistency() {
        // All these should produce the same result: "0100"
        assert_eq!(
            TimeHandler::validate_and_canonicalize("1", "time").unwrap(),
            "0100"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("01", "time").unwrap(),
            "0100"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("1:00", "time").unwrap(),
            "0100"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("0100", "time").unwrap(),
            "0100"
        );

        // All these should produce the same result: "1430"
        assert_eq!(
            TimeHandler::validate_and_canonicalize("14:30", "time").unwrap(),
            "1430"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("1430", "time").unwrap(),
            "1430"
        );

        // All these should produce the same result: "0000"
        assert_eq!(
            TimeHandler::validate_and_canonicalize("0", "time").unwrap(),
            "0000"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("00", "time").unwrap(),
            "0000"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("0:00", "time").unwrap(),
            "0000"
        );
        assert_eq!(
            TimeHandler::validate_and_canonicalize("0000", "time").unwrap(),
            "0000"
        );
    }
}
