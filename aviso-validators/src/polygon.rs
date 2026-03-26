// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use anyhow::{Result, bail};
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

        // Parse the coordinate string
        let coordinates = Self::parse_polygon_coordinates(value)?;
        debug!(
            "Parsed {} coordinate pairs for field '{}'",
            coordinates.len(),
            field_name
        );

        // Validate the polygon
        Self::validate_polygon_geometry(&coordinates)?;
        debug!(
            "Polygon geometry validation passed for field '{}'",
            field_name
        );

        // Return the original validated string
        // (JSON conversion will happen elsewhere when building the payload)
        Ok(value.to_string())
    }

    /// Parse a string of coordinates "(lat,lon,lat,lon,...)" into a vector of (lat, lon) tuples.
    ///
    /// This function ALWAYS returns (lat, lon)
    /// DO NOT swap here. Only swap to (lon, lat) when passing to geo crate.
    pub fn parse_polygon_coordinates(coord_string: &str) -> Result<Vec<(f64, f64)>> {
        let trimmed = coord_string
            .trim()
            .trim_start_matches('(')
            .trim_end_matches(')')
            .trim();

        if trimmed.is_empty() {
            bail!("Empty polygon coordinate string");
        }

        let coord_parts: Vec<&str> = trimmed.split(',').collect();

        if !coord_parts.len().is_multiple_of(2) {
            bail!("Polygon coordinates must be in pairs (lat,lon)");
        }

        let mut coordinates = Vec::new();
        let mut iter = coord_parts.iter();

        while let Some(lat_str) = iter.next() {
            let lon_str = iter.next().unwrap(); // Already checked length above

            let lat: f64 = lat_str
                .trim()
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid latitude value: {}", lat_str))?;

            let lon: f64 = lon_str
                .trim()
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid longitude value: {}", lon_str))?;

            coordinates.push((lat, lon));
        }

        Ok(coordinates)
    }

    /// Validates polygon geometry requirements
    fn validate_polygon_geometry(coordinates: &[(f64, f64)]) -> Result<()> {
        if coordinates.len() < 3 {
            bail!("Polygon must have at least 3 coordinate pairs");
        }

        // Check if polygon is closed (first and last coordinates are the same)
        let first = coordinates.first().unwrap();
        let last = coordinates.last().unwrap();

        if first != last {
            bail!("Polygon must be closed (first and last coordinates must be identical)");
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
