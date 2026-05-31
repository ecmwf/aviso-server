// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use anyhow::{Result, anyhow, bail};
use tracing::debug;

/// Polygon coordinate validator
///
/// Validates polygon coordinate strings in the format "(lat1,lon1,lat2,lon2,lat1,lon1)"
/// and ensures proper polygon geometry (closed, minimum vertices, valid coordinates).
pub struct PolygonHandler;

impl PolygonHandler {
    pub fn validate_and_canonicalize(value: &str, field_name: &str) -> Result<String> {
        debug!(
            "Validating polygon field '{}' with value: {}",
            field_name, value
        );

        let coordinates = Self::parse_polygon_coordinates(value)
            .map_err(|e| anyhow!("field '{}' must be a valid polygon: {}", field_name, e))?;
        debug!(
            "Parsed {} coordinate pairs for field '{}'",
            coordinates.len(),
            field_name
        );

        Self::validate_polygon_geometry(&coordinates)
            .map_err(|e| anyhow!("field '{}' must be a valid polygon: {}", field_name, e))?;
        debug!(
            "Polygon geometry validation passed for field '{}'",
            field_name
        );

        Ok(value.to_string())
    }

    /// Parse a polygon coordinate string into a vector of `(lat, lon)` tuples.
    ///
    /// Accepted forms (whitespace tolerated everywhere):
    ///   * `"(lat1,lon1,...,lat1,lon1)"` — parenthesised, balanced
    ///   * `"lat1,lon1,...,lat1,lon1"`   — no parentheses
    ///
    /// Rejected forms (each with a specific error message):
    ///   * Opening `(` without a matching closing `)` (or vice versa)
    ///   * Embedded `(` or `)` anywhere except as the single outer pair
    ///   * Empty string or `()`
    ///   * Odd number of comma-separated values
    ///   * Any value that does not parse as `f64`
    ///
    /// This function ALWAYS returns `(lat, lon)` pairs. DO NOT swap here; only
    /// swap to `(lon, lat)` when passing to the `geo` crate.
    pub fn parse_polygon_coordinates(coord_string: &str) -> Result<Vec<(f64, f64)>> {
        let raw = coord_string.trim();
        if raw.is_empty() {
            bail!("polygon coordinate string is empty");
        }

        let inner = match (raw.starts_with('('), raw.ends_with(')')) {
            (true, true) => &raw[1..raw.len() - 1],
            (false, false) => raw,
            (true, false) => {
                bail!("polygon coordinate string has opening '(' but is missing the closing ')'")
            }
            (false, true) => {
                bail!("polygon coordinate string has closing ')' but is missing the opening '('")
            }
        };

        if inner.contains('(') || inner.contains(')') {
            bail!(
                "polygon coordinate string must have at most one outer pair of parentheses; \
                 nested '(' or ')' are not allowed"
            );
        }

        let inner = inner.trim();
        if inner.is_empty() {
            bail!("polygon coordinate string is empty between parentheses");
        }

        let coord_parts: Vec<&str> = inner.split(',').collect();

        if !coord_parts.len().is_multiple_of(2) {
            bail!("polygon coordinates must be in lat,lon pairs (got an odd number of values)");
        }

        let mut coordinates = Vec::new();
        let mut iter = coord_parts.iter();

        while let Some(lat_str) = iter.next() {
            let lon_str = iter.next().unwrap(); // Already checked length above

            let lat: f64 = lat_str.trim().parse().map_err(|_| {
                anyhow!("could not parse latitude '{}' as a number", lat_str.trim())
            })?;

            let lon: f64 = lon_str.trim().parse().map_err(|_| {
                anyhow!("could not parse longitude '{}' as a number", lon_str.trim())
            })?;

            // Range check after parse. `RangeInclusive::contains` uses f64's
            // PartialOrd comparisons, so this single guard rejects NaN (every
            // ordering comparison against NaN is false) and rejects ±inf (out
            // of any finite range) without a separate is_finite() check.
            if !(-90.0..=90.0).contains(&lat) {
                bail!("latitude {} is outside the valid range [-90, 90]", lat);
            }
            if !(-180.0..=180.0).contains(&lon) {
                bail!("longitude {} is outside the valid range [-180, 180]", lon);
            }

            coordinates.push((lat, lon));
        }

        Ok(coordinates)
    }

    /// Validates polygon geometry requirements
    fn validate_polygon_geometry(coordinates: &[(f64, f64)]) -> Result<()> {
        if coordinates.len() < 4 {
            bail!(
                "polygon must have at least 4 coordinate pairs (3 unique vertices plus a \
                 closing repeat of the first vertex)"
            );
        }

        let first = coordinates.first().unwrap();
        let last = coordinates.last().unwrap();

        if first != last {
            bail!("polygon must be closed (first and last coordinates must be identical)");
        }

        Ok(())
    }

    /// Calculates bounding box for spatial filtering optimization
    /// This will be used later when we handle the payload and headers
    pub fn calculate_bounding_box(coordinates: &[(f64, f64)]) -> String {
        let mut min_lat = f64::INFINITY;
        let mut min_lon = f64::INFINITY;
        let mut max_lat = f64::NEG_INFINITY;
        let mut max_lon = f64::NEG_INFINITY;

        for &(lat, lon) in coordinates {
            min_lat = min_lat.min(lat);
            min_lon = min_lon.min(lon);
            max_lat = max_lat.max(lat);
            max_lon = max_lon.max(lon);
        }

        format!("{},{},{},{}", min_lat, min_lon, max_lat, max_lon)
    }

    pub fn parse_bbox_coordinates(s: &str) -> Result<(f64, f64, f64, f64)> {
        // expects (lat_min,lon_min,lat_max,lon_max)
        let s = s.trim_matches(|c| c == '(' || c == ')');
        let coords: Vec<f64> = s
            .split(',')
            .map(|part| part.trim().parse())
            .collect::<Result<_, _>>()?;
        if coords.len() != 4 {
            anyhow::bail!("BBox must have 4 numbers");
        }
        Ok((coords[0], coords[1], coords[2], coords[3]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_and_canonicalize_valid_polygon() {
        let polygon_str = "(52.5,13.4,52.6,13.5,52.5,13.6,52.4,13.5,52.5,13.4)";
        let result = PolygonHandler::validate_and_canonicalize(polygon_str, "polygon");

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), polygon_str);
    }

    #[test]
    fn test_validate_and_canonicalize_with_spaces() {
        let polygon_str = "( 52.5 , 13.4 , 52.6 , 13.5 , 52.5 , 13.6 , 52.4 , 13.5 , 52.5 , 13.4 )";
        let result = PolygonHandler::validate_and_canonicalize(polygon_str, "polygon");

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), polygon_str);
    }

    #[test]
    fn test_validate_and_canonicalize_not_closed() {
        let polygon_str = "(52.5,13.4,52.6,13.5,52.5,13.6,52.4,13.5)"; // Missing closing point
        let result = PolygonHandler::validate_and_canonicalize(polygon_str, "polygon");

        assert!(result.is_err());
    }

    #[test]
    fn test_validate_and_canonicalize_too_few_points() {
        let polygon_str = "(52.5,13.4,52.6,13.5)"; // Only 2 points
        let result = PolygonHandler::validate_and_canonicalize(polygon_str, "polygon");

        assert!(result.is_err());
    }

    #[test]
    fn test_validate_and_canonicalize_empty_string() {
        let polygon_str = "";
        let result = PolygonHandler::validate_and_canonicalize(polygon_str, "polygon");

        assert!(result.is_err());
    }

    #[test]
    fn test_validate_and_canonicalize_empty_parentheses() {
        let polygon_str = "()";
        let result = PolygonHandler::validate_and_canonicalize(polygon_str, "polygon");

        assert!(result.is_err());
    }

    #[test]
    fn test_parse_polygon_coordinates_valid() {
        let coord_string = "(52.5,13.4,52.6,13.5,52.5,13.6,52.4,13.5,52.5,13.4)";
        let result = PolygonHandler::parse_polygon_coordinates(coord_string);

        assert!(result.is_ok());
        let coordinates = result.unwrap();
        assert_eq!(coordinates.len(), 5);
        assert_eq!(coordinates[0], (52.5, 13.4));
        assert_eq!(coordinates[1], (52.6, 13.5));
        assert_eq!(coordinates[4], (52.5, 13.4)); // Should be closed
    }

    #[test]
    fn test_parse_polygon_coordinates_without_parentheses() {
        let coord_string = "52.5,13.4,52.6,13.5,52.5,13.6,52.4,13.5,52.5,13.4";
        let result = PolygonHandler::parse_polygon_coordinates(coord_string);

        assert!(result.is_ok());
        let coordinates = result.unwrap();
        assert_eq!(coordinates.len(), 5);
        assert_eq!(coordinates[0], (52.5, 13.4));
    }

    #[test]
    fn test_parse_polygon_coordinates_with_spaces() {
        let coord_string = "( 52.5 , 13.4 , 52.6 , 13.5 , 52.5 , 13.4 )";
        let result = PolygonHandler::parse_polygon_coordinates(coord_string);

        assert!(result.is_ok());
        let coordinates = result.unwrap();
        assert_eq!(coordinates.len(), 3);
        assert_eq!(coordinates[0], (52.5, 13.4));
        assert_eq!(coordinates[1], (52.6, 13.5));
        assert_eq!(coordinates[2], (52.5, 13.4));
    }

    #[test]
    fn test_parse_polygon_coordinates_odd_number() {
        let coord_string = "(52.5,13.4,52.6)"; // Odd number of coordinates
        let result = PolygonHandler::parse_polygon_coordinates(coord_string);

        assert!(result.is_err());
    }

    #[test]
    fn test_parse_polygon_coordinates_invalid_latitude() {
        let coord_string = "(invalid,13.4,52.6,13.5,52.5,13.4)";
        let result = PolygonHandler::parse_polygon_coordinates(coord_string);

        assert!(result.is_err());
    }

    #[test]
    fn test_parse_polygon_coordinates_invalid_longitude() {
        let coord_string = "(52.5,invalid,52.6,13.5,52.5,13.4)";
        let result = PolygonHandler::parse_polygon_coordinates(coord_string);

        assert!(result.is_err());
    }

    #[test]
    fn test_parse_polygon_coordinates_empty() {
        let coord_string = "()";
        let result = PolygonHandler::parse_polygon_coordinates(coord_string);

        assert!(result.is_err());
    }

    #[test]
    fn rejects_polygon_with_opening_paren_but_no_closing_paren() {
        let coord_string = "(50.0,10.0,52.0,10.0,52.0,12.0,50.0,12.0,50.0,10.0";
        let err = PolygonHandler::parse_polygon_coordinates(coord_string)
            .expect_err("unbalanced parens must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("opening") && msg.contains("missing the closing"),
            "error should pinpoint the missing closing paren; got: {msg}"
        );
    }

    #[test]
    fn rejects_polygon_with_closing_paren_but_no_opening_paren() {
        let coord_string = "50.0,10.0,52.0,10.0,52.0,12.0,50.0,12.0,50.0,10.0)";
        let err = PolygonHandler::parse_polygon_coordinates(coord_string)
            .expect_err("unbalanced parens must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("closing") && msg.contains("missing the opening"),
            "error should pinpoint the missing opening paren; got: {msg}"
        );
    }

    #[test]
    fn rejects_polygon_with_extra_nested_parens() {
        let coord_string = "(50.0,10.0),52.0,10.0,52.0,12.0,50.0,12.0,50.0,10.0)";
        let err = PolygonHandler::parse_polygon_coordinates(coord_string)
            .expect_err("nested parens must be rejected, not produce a confusing parse error");
        let msg = err.to_string();
        assert!(
            msg.contains("nested") || msg.contains("outer pair"),
            "error should mention parentheses placement, not e.g. a number-parse failure; got: {msg}"
        );
    }

    #[test]
    fn rejects_latitude_above_90() {
        let coord_string = "(91.0,10.0,50.0,10.0,50.0,11.0,91.0,10.0)";
        let err = PolygonHandler::parse_polygon_coordinates(coord_string)
            .expect_err("latitude > 90 must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("latitude") && msg.contains("-90") && msg.contains("90"),
            "error should pinpoint the latitude range; got: {msg}"
        );
    }

    #[test]
    fn rejects_latitude_below_minus_90() {
        let coord_string = "(-90.5,10.0,50.0,10.0,50.0,11.0,-90.5,10.0)";
        let err = PolygonHandler::parse_polygon_coordinates(coord_string)
            .expect_err("latitude < -90 must be rejected");
        let msg = err.to_string();
        assert!(msg.contains("latitude") && msg.contains("-90.5"));
    }

    #[test]
    fn rejects_longitude_above_180() {
        let coord_string = "(50.0,181.0,50.0,10.0,50.0,11.0,50.0,181.0)";
        let err = PolygonHandler::parse_polygon_coordinates(coord_string)
            .expect_err("longitude > 180 must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("longitude") && msg.contains("-180") && msg.contains("180"),
            "error should pinpoint the longitude range; got: {msg}"
        );
    }

    #[test]
    fn rejects_longitude_below_minus_180() {
        let coord_string = "(50.0,-180.5,50.0,10.0,50.0,11.0,50.0,-180.5)";
        let err = PolygonHandler::parse_polygon_coordinates(coord_string)
            .expect_err("longitude < -180 must be rejected");
        let msg = err.to_string();
        assert!(msg.contains("longitude") && msg.contains("-180.5"));
    }

    #[test]
    fn rejects_non_finite_coordinates_via_range_check() {
        // NaN and ±inf parse as valid f64 but fall outside any finite range, so
        // the existing range check rejects them without a separate is_finite() guard.
        for bad_value in &["NaN", "inf", "-inf"] {
            let coord_string = format!("({bad_value},10.0,50.0,10.0,50.0,11.0,{bad_value},10.0)");
            assert!(
                PolygonHandler::parse_polygon_coordinates(&coord_string).is_err(),
                "non-finite coordinate `{bad_value}` must be rejected"
            );
        }
    }

    #[test]
    fn accepts_exact_boundary_coordinates() {
        let polygon = "(90.0,-180.0,89.0,-180.0,89.0,-179.0,90.0,-180.0)";
        let result = PolygonHandler::parse_polygon_coordinates(polygon);
        assert!(
            result.is_ok(),
            "lat=90 and lon=-180 are valid (inclusive) boundary values: {:?}",
            result.err()
        );
    }

    #[test]
    fn validate_and_canonicalize_wraps_errors_with_field_name_and_validation_marker() {
        // The classifier in handlers::notification_processor matches "field '" and
        // "must be a valid" to route polygon errors to a 400 response. This test
        // pins both substrings so the public error-classification contract does
        // not silently drift.
        let bad = "(50.0,10.0,52.0,10.0,52.0,12.0,50.0,12.0,50.0,10.0";
        let err = PolygonHandler::validate_and_canonicalize(bad, "polygon")
            .expect_err("unbalanced polygon must error");
        let msg = err.to_string();
        assert!(
            msg.contains("field 'polygon'") && msg.contains("must be a valid"),
            "error must carry the validation-classifier markers; got: {msg}"
        );
    }

    #[test]
    fn test_validate_polygon_geometry_valid_triangle() {
        let coordinates = vec![(0.0, 0.0), (1.0, 0.0), (0.5, 1.0), (0.0, 0.0)];
        let result = PolygonHandler::validate_polygon_geometry(&coordinates);

        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_polygon_geometry_valid_rectangle() {
        let coordinates = vec![
            (52.5, 13.4),
            (52.6, 13.4),
            (52.6, 13.5),
            (52.5, 13.5),
            (52.5, 13.4),
        ];
        let result = PolygonHandler::validate_polygon_geometry(&coordinates);

        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_polygon_geometry_too_few_points() {
        let coordinates = vec![(0.0, 0.0), (1.0, 0.0)]; // Only 2 points
        let result = PolygonHandler::validate_polygon_geometry(&coordinates);

        assert!(result.is_err());
    }

    #[test]
    fn rejects_three_pair_closed_line_segment_as_degenerate_polygon() {
        // Three pairs is two unique vertices closed back on the first, i.e. a line
        // segment, not a polygon. The downstream geo conversion in
        // src/notification/spatial.rs requires 4+ pairs; without rejecting here
        // the request silently degraded to a 500 NOTIFICATION_PROCESSING_FAILED.
        let coordinates = vec![(0.0, 0.0), (1.0, 0.0), (0.0, 0.0)];
        let err = PolygonHandler::validate_polygon_geometry(&coordinates)
            .expect_err("3-pair closed line segment must be rejected as a polygon");
        let msg = err.to_string();
        assert!(
            msg.contains("at least 4 coordinate pairs"),
            "error must specify the new minimum; got: {msg}"
        );
    }

    #[test]
    fn test_validate_polygon_geometry_not_closed() {
        let coordinates = vec![(0.0, 0.0), (1.0, 0.0), (0.5, 1.0), (0.1, 0.1)]; // Not closed
        let result = PolygonHandler::validate_polygon_geometry(&coordinates);

        assert!(result.is_err());
    }

    #[test]
    fn test_validate_polygon_geometry_minimum_valid() {
        let coordinates = vec![(0.0, 0.0), (1.0, 0.0), (0.5, 1.0), (0.0, 0.0)]; // Minimum valid triangle
        let result = PolygonHandler::validate_polygon_geometry(&coordinates);

        assert!(result.is_ok());
    }

    #[test]
    fn test_calculate_bounding_box_rectangle() {
        let coordinates = vec![
            (52.5, 13.4),
            (52.6, 13.4),
            (52.6, 13.5),
            (52.5, 13.5),
            (52.5, 13.4),
        ];
        let bbox = PolygonHandler::calculate_bounding_box(&coordinates);

        assert_eq!(bbox, "52.5,13.4,52.6,13.5");
    }

    #[test]
    fn test_calculate_bounding_box_triangle() {
        let coordinates = vec![(0.0, 0.0), (1.0, 0.0), (0.5, 1.0), (0.0, 0.0)];
        let bbox = PolygonHandler::calculate_bounding_box(&coordinates);

        assert_eq!(bbox, "0,0,1,1");
    }

    #[test]
    fn test_calculate_bounding_box_single_point() {
        let coordinates = vec![(52.5, 13.4), (52.5, 13.4), (52.5, 13.4), (52.5, 13.4)];
        let bbox = PolygonHandler::calculate_bounding_box(&coordinates);

        assert_eq!(bbox, "52.5,13.4,52.5,13.4");
    }

    #[test]
    fn test_calculate_bounding_box_negative_coordinates() {
        let coordinates = vec![
            (-1.0, -1.0),
            (1.0, -1.0),
            (1.0, 1.0),
            (-1.0, 1.0),
            (-1.0, -1.0),
        ];
        let bbox = PolygonHandler::calculate_bounding_box(&coordinates);

        assert_eq!(bbox, "-1,-1,1,1");
    }

    #[test]
    fn test_integration_parse_and_validate() {
        let polygon_str = "(52.5,13.4,52.6,13.5,52.5,13.6,52.4,13.5,52.5,13.4)";

        // Test the full pipeline: parse -> validate -> calculate bbox
        let coordinates = PolygonHandler::parse_polygon_coordinates(polygon_str).unwrap();
        let validation_result = PolygonHandler::validate_polygon_geometry(&coordinates);
        assert!(validation_result.is_ok());

        let bbox = PolygonHandler::calculate_bounding_box(&coordinates);
        assert_eq!(bbox, "52.4,13.4,52.6,13.6");
    }

    #[test]
    fn test_real_world_berlin_polygon() {
        // Real-world coordinates around Berlin
        let polygon_str =
            "(52.5200,13.4050,52.5200,13.4500,52.4800,13.4500,52.4800,13.4050,52.5200,13.4050)";
        let result = PolygonHandler::validate_and_canonicalize(polygon_str, "berlin_area");

        assert!(result.is_ok());

        let coordinates = PolygonHandler::parse_polygon_coordinates(polygon_str).unwrap();
        let bbox = PolygonHandler::calculate_bounding_box(&coordinates);
        assert_eq!(bbox, "52.48,13.405,52.52,13.45");
    }

    #[test]
    fn test_precision_handling() {
        // Test with high precision coordinates
        let polygon_str = "(52.123456789,13.987654321,52.234567890,13.876543210,52.345678901,13.765432109,52.123456789,13.987654321)";
        let result = PolygonHandler::validate_and_canonicalize(polygon_str, "precision_test");

        assert!(result.is_ok());

        let coordinates = PolygonHandler::parse_polygon_coordinates(polygon_str).unwrap();
        assert_eq!(coordinates[0].0, 52.123456789);
        assert_eq!(coordinates[0].1, 13.987654321);
    }
}
