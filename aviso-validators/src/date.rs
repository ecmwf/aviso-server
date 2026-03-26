// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Date validation and canonicalization handler
//!
//! Handles validation and canonicalization of date fields according to various
//! input formats. Supports multiple common date representations and converts
//! them to a consistent canonical format for topic generation and storage.

use anyhow::{Context, Result, bail};
use chrono::NaiveDate;

/// Date validation and canonicalization handler
///
/// Supports multiple input date formats:
/// - **YYYY-MM-DD**: ISO 8601 standard format (e.g., "2025-12-25")
/// - **YYYYMMDD**: Compact format without separators (e.g., "20251225")
/// - **YYYY-DDD**: Day-of-year format (e.g., "2025-359" for December 25th)
///
/// All valid input formats are canonicalized to the format specified in the
/// schema configuration, ensuring consistent representation across the system.
pub struct DateHandler;

impl DateHandler {
    /// Validate and canonicalize a date value according to schema requirements
    ///
    /// This method performs comprehensive date validation:
    /// 1. Attempts to parse the input using multiple supported formats
    /// 2. Validates that the date is actually valid (e.g., no February 30th)
    /// 3. Converts to the canonical format specified in the schema
    ///
    /// # Arguments
    /// * `value` - The date string to validate (in any supported format)
    /// * `canonical_format` - Target format for canonicalization ("%Y%m%d" or "%Y-%m-%d")
    /// * `field_name` - Name of the field being validated (for error messages)
    ///
    /// # Returns
    /// * `Ok(String)` - The date in canonical format
    /// * `Err(anyhow::Error)` - Invalid date format or impossible date
    pub fn validate_and_canonicalize(
        value: &str,
        canonical_format: &str,
        field_name: &str,
    ) -> Result<String> {
        // Parse the input date using our flexible parser
        let parsed_date = Self::parse_date(value, field_name).context(format!(
            "Failed to parse date value for field '{}'",
            field_name
        ))?;

        // Convert to the requested canonical format
        let canonicalized = match canonical_format {
            "%Y%m%d" => parsed_date.format("%Y%m%d").to_string(),
            "%Y-%m-%d" => parsed_date.format("%Y-%m-%d").to_string(),
            _ => bail!(
                "Unsupported date format '{}' for field '{}'",
                canonical_format,
                field_name
            ),
        };

        tracing::debug!(
            field_name = field_name,
            input_value = value,
            canonical_value = %canonicalized,
            canonical_format = canonical_format,
            "Date successfully validated and canonicalized"
        );

        Ok(canonicalized)
    }

    /// Parse date from multiple supported input formats
    ///
    /// Attempts to parse the input string using various common date formats
    /// in order of preference. This flexible approach allows clients to use
    /// whatever date format is most convenient for their use case.
    ///
    /// # Arguments
    /// * `value` - The date string to parse
    /// * `field_name` - Name of the field (for error messages)
    ///
    /// # Returns
    /// * `Ok(NaiveDate)` - Successfully parsed date
    /// * `Err(anyhow::Error)` - No supported format could parse the input
    ///
    /// # Supported Formats
    /// 1. **ISO 8601**: YYYY-MM-DD (e.g., "2025-12-25")
    /// 2. **Compact**: YYYYMMDD (e.g., "20251225")
    /// 3. **Day-of-year**: YYYY-DDD (e.g., "2025-359")
    fn parse_date(value: &str, field_name: &str) -> Result<NaiveDate> {
        // Try ISO 8601 format first (YYYY-MM-DD)
        if let Ok(date) = NaiveDate::parse_from_str(value, "%Y-%m-%d") {
            tracing::debug!(
                field_name = field_name,
                input_value = value,
                parsed_format = "ISO 8601 (YYYY-MM-DD)",
                "Date parsed successfully"
            );
            return Ok(date);
        }

        // Try compact format (YYYYMMDD)
        if let Ok(date) = NaiveDate::parse_from_str(value, "%Y%m%d") {
            tracing::debug!(
                field_name = field_name,
                input_value = value,
                parsed_format = "Compact (YYYYMMDD)",
                "Date parsed successfully"
            );
            return Ok(date);
        }

        // Try day-of-year format (YYYY-DDD)
        if let Ok(date) = NaiveDate::parse_from_str(value, "%Y-%j") {
            tracing::debug!(
                field_name = field_name,
                input_value = value,
                parsed_format = "Day-of-year (YYYY-DDD)",
                "Date parsed successfully"
            );
            return Ok(date);
        }

        // No format worked, provide comprehensive error message
        bail!(
            "Field '{}' contains invalid date '{}'. Expected: YYYY-MM-DD, YYYYMMDD, or YYYY-DDD",
            field_name,
            value
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_iso_8601_format() {
        let result = DateHandler::validate_and_canonicalize("2025-12-25", "%Y%m%d", "date");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "20251225");
    }

    #[test]
    fn test_compact_format() {
        let result = DateHandler::validate_and_canonicalize("20251225", "%Y-%m-%d", "date");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "2025-12-25");
    }

    #[test]
    fn test_day_of_year_format() {
        let result = DateHandler::validate_and_canonicalize("2025-359", "%Y%m%d", "date");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "20251225"); // 359th day of 2025 is December 25th
    }

    #[test]
    fn test_invalid_date() {
        let result = DateHandler::validate_and_canonicalize("2025-02-30", "%Y%m%d", "date");
        assert!(result.is_err());
    }

    #[test]
    fn test_unsupported_canonical_format() {
        let result = DateHandler::validate_and_canonicalize("2025-12-25", "%d/%m/%Y", "date");
        assert!(result.is_err());
    }

    #[test]
    fn test_leap_year_handling() {
        // Test February 29th in leap year (2025)
        let result = DateHandler::validate_and_canonicalize("2024-02-29", "%Y%m%d", "date");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "20240229");

        // Test February 29th in non-leap year (2023)
        let result = DateHandler::validate_and_canonicalize("2023-02-29", "%Y%m%d", "date");
        assert!(result.is_err());
    }

    #[test]
    fn test_date_boundary_conditions() {
        // Test year boundaries
        let result = DateHandler::validate_and_canonicalize("1999-12-31", "%Y%m%d", "date");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "19991231");

        let result = DateHandler::validate_and_canonicalize("2000-01-01", "%Y%m%d", "date");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "20000101");

        // Test month boundaries
        let result = DateHandler::validate_and_canonicalize("2025-01-31", "%Y%m%d", "date");
        assert!(result.is_ok());

        let result = DateHandler::validate_and_canonicalize("2025-02-01", "%Y%m%d", "date");
        assert!(result.is_ok());
    }

    #[test]
    fn test_invalid_dates_comprehensive() {
        // Invalid month
        let result = DateHandler::validate_and_canonicalize("2025-13-01", "%Y%m%d", "date");
        assert!(result.is_err());

        // Invalid day
        let result = DateHandler::validate_and_canonicalize("2025-01-32", "%Y%m%d", "date");
        assert!(result.is_err());

        // February 30th
        let result = DateHandler::validate_and_canonicalize("2025-02-30", "%Y%m%d", "date");
        assert!(result.is_err());

        // April 31st (April has only 30 days)
        let result = DateHandler::validate_and_canonicalize("2025-04-31", "%Y%m%d", "date");
        assert!(result.is_err());
    }

    #[test]
    fn test_day_of_year_edge_cases() {
        // Day 1 of year
        assert_eq!(
            DateHandler::validate_and_canonicalize("2025-001", "%Y%m%d", "date").unwrap(),
            "20250101"
        );
        // Day 365 of non-leap year
        assert_eq!(
            DateHandler::validate_and_canonicalize("2025-365", "%Y%m%d", "date").unwrap(),
            "20251231"
        );
        // Day 366 of leap year
        assert_eq!(
            DateHandler::validate_and_canonicalize("2024-366", "%Y%m%d", "date").unwrap(),
            "20241231"
        );
        // Day 366 of non-leap year should fail
        assert!(DateHandler::validate_and_canonicalize("2025-366", "%Y%m%d", "date").is_err());
        // Day 0 should fail
        assert!(DateHandler::validate_and_canonicalize("2025-000", "%Y%m%d", "date").is_err());
    }

    #[test]
    fn test_format_consistency() {
        // Same date in different formats should produce same canonical result
        let iso_result =
            DateHandler::validate_and_canonicalize("2025-12-25", "%Y%m%d", "date").unwrap();
        let compact_result =
            DateHandler::validate_and_canonicalize("20251225", "%Y%m%d", "date").unwrap();
        let doy_result =
            DateHandler::validate_and_canonicalize("2025-359", "%Y%m%d", "date").unwrap();

        assert_eq!(iso_result, compact_result);
        assert_eq!(compact_result, doy_result);
        assert_eq!(iso_result, "20251225");
    }

    #[test]
    fn test_malformed_input_formats() {
        let malformed_inputs = [
            "2025/12/25",          // wrong separator (slash)
            "2025.12.25",          // wrong separator (dot)
            "2025",                // incomplete (year only)
            "2025-12-25T00:00:00", // extra time part
            "25-12-2025",          // wrong order
            "2025-13-01",          // invalid month (13)
            "2025-02-30",          // invalid date (Feb 30)
            "not-a-date",          // completely invalid
            "",                    // empty string
            "2025-",               // incomplete with separator
            "abc-def-ghi",         // non-numeric
        ];

        for input in malformed_inputs {
            let result = DateHandler::validate_and_canonicalize(input, "%Y%m%d", "date");
            assert!(
                result.is_err(),
                "Should fail for input: '{}', but got: {:?}",
                input,
                result
            );
        }
    }
}
