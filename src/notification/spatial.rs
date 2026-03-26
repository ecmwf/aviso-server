// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Spatial helpers for polygon notifications.

use anyhow::Result;
use geo::{BoundingRect, Intersects};
use geo_types::{Polygon, Rect};
use serde::{Deserialize, Serialize};
use tracing::debug;

/// Spatial metadata derived from polygon fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpatialMetadata {
    /// Bounding box string: `min_lat,min_lon,max_lat,max_lon`.
    pub bounding_box: String,
}

impl SpatialMetadata {
    /// Build metadata from polygon coordinates.
    pub fn from_coordinates(coordinates: &[(f64, f64)]) -> Result<Self> {
        let bounding_box = calculate_bounding_box_string(coordinates)?;
        Ok(Self { bounding_box })
    }

    /// Parse bounding box to `geo_types::Rect`.
    pub fn as_geo_rect(&self) -> Result<Rect<f64>> {
        bounding_box_string_to_geo_rect(&self.bounding_box)
    }

    /// Serialize metadata to JSON.
    pub fn to_json_string(&self) -> Result<String> {
        serde_json::to_string(self)
            .map_err(|e| anyhow::anyhow!("Failed to serialize spatial metadata: {}", e))
    }

    /// Deserialize metadata from JSON.
    pub fn from_json_string(json_str: &str) -> Result<Self> {
        serde_json::from_str(json_str)
            .map_err(|e| anyhow::anyhow!("Failed to deserialize spatial metadata: {}", e))
    }
}

/// Calculate bounding box string from polygon coordinates.
pub fn calculate_bounding_box_string(coordinates: &[(f64, f64)]) -> Result<String> {
    if coordinates.is_empty() {
        anyhow::bail!("Cannot calculate bounding box for empty coordinates");
    }

    let polygon = coordinates_to_geo_polygon(coordinates)?;

    let bounding_rect = polygon
        .bounding_rect()
        .ok_or_else(|| anyhow::anyhow!("Failed to calculate bounding rectangle for polygon"))?;

    Ok(format!(
        "{},{},{},{}",
        bounding_rect.min().y, // min_lat
        bounding_rect.min().x, // min_lon
        bounding_rect.max().y, // max_lat
        bounding_rect.max().x  // max_lon
    ))
}

/// Convert `(lat, lon)` pairs to a geo polygon.
pub fn coordinates_to_geo_polygon(coordinates: &[(f64, f64)]) -> Result<Polygon<f64>> {
    if coordinates.len() < 4 {
        anyhow::bail!("Polygon must have at least 4 coordinate pairs (including closure)");
    }

    // Geo uses (x, y) = (lon, lat).
    let geo_coords: Vec<(f64, f64)> = coordinates.iter().map(|(lat, lon)| (*lon, *lat)).collect();

    let polygon = Polygon::new(geo_coords.into(), vec![]);

    debug!(
        coordinate_count = coordinates.len(),
        "Converted coordinates to geo polygon"
    );

    Ok(polygon)
}

/// Parse bounding box string to geo rectangle.
pub fn bounding_box_string_to_geo_rect(bbox_string: &str) -> Result<Rect<f64>> {
    let parts: Vec<&str> = bbox_string.split(',').collect();

    if parts.len() != 4 {
        anyhow::bail!("Bounding box must have 4 components: min_lat,min_lon,max_lat,max_lon");
    }

    let min_lat: f64 = parts[0]
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid min_lat in bounding box: {}", e))?;
    let min_lon: f64 = parts[1]
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid min_lon in bounding box: {}", e))?;
    let max_lat: f64 = parts[2]
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid max_lat in bounding box: {}", e))?;
    let max_lon: f64 = parts[3]
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid max_lon in bounding box: {}", e))?;

    // Geo expects (x, y) = (lon, lat).
    let rect = Rect::new((min_lon, min_lat), (max_lon, max_lat));

    Ok(rect)
}

/// Test whether two bounding boxes intersect.
pub fn bounding_boxes_intersect(bbox1: &str, bbox2: &str) -> Result<bool> {
    let rect1 = bounding_box_string_to_geo_rect(bbox1)?;
    let rect2 = bounding_box_string_to_geo_rect(bbox2)?;

    Ok(rect1.intersects(&rect2))
}

/// Test whether two polygons intersect.
pub fn polygons_intersect(
    polygon1_coords: &[(f64, f64)],
    polygon2_coords: &[(f64, f64)],
) -> Result<bool> {
    let polygon1 = coordinates_to_geo_polygon(polygon1_coords)?;
    let polygon2 = coordinates_to_geo_polygon(polygon2_coords)?;

    Ok(polygon1.intersects(&polygon2))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test data - Berlin area polygon
    fn berlin_polygon() -> Vec<(f64, f64)> {
        vec![
            (52.5, 13.4), // Southwest
            (52.6, 13.4), // Northwest
            (52.6, 13.5), // Northeast
            (52.5, 13.5), // Southeast
            (52.5, 13.4), // Close the polygon
        ]
    }

    // Test data - Simple triangle
    fn triangle_polygon() -> Vec<(f64, f64)> {
        vec![
            (0.0, 0.0),
            (1.0, 0.0),
            (0.5, 1.0),
            (0.0, 0.0), // Close the polygon
        ]
    }

    // Test data - Overlapping polygon with Berlin
    fn overlapping_polygon() -> Vec<(f64, f64)> {
        vec![
            (52.55, 13.45), // Overlaps with Berlin polygon
            (52.65, 13.45),
            (52.65, 13.55),
            (52.55, 13.55),
            (52.55, 13.45),
        ]
    }

    // Test data - Non-overlapping polygon
    fn distant_polygon() -> Vec<(f64, f64)> {
        vec![
            (50.0, 10.0),
            (50.1, 10.0),
            (50.1, 10.1),
            (50.0, 10.1),
            (50.0, 10.0),
        ]
    }

    #[test]
    fn test_spatial_metadata_from_coordinates() {
        let coordinates = berlin_polygon();
        let result = SpatialMetadata::from_coordinates(&coordinates);

        assert!(result.is_ok());
        let metadata = result.unwrap();
        assert!(!metadata.bounding_box.is_empty());

        // Should have format: min_lat,min_lon,max_lat,max_lon
        let parts: Vec<&str> = metadata.bounding_box.split(',').collect();
        assert_eq!(parts.len(), 4);

        // Verify bounding box values for Berlin polygon
        assert_eq!(parts[0], "52.5"); // min_lat
        assert_eq!(parts[1], "13.4"); // min_lon
        assert_eq!(parts[2], "52.6"); // max_lat
        assert_eq!(parts[3], "13.5"); // max_lon
    }

    #[test]
    fn test_spatial_metadata_from_empty_coordinates() {
        let coordinates = vec![];
        let result = SpatialMetadata::from_coordinates(&coordinates);

        assert!(result.is_err());
    }

    #[test]
    fn test_spatial_metadata_as_geo_rect() {
        let coordinates = berlin_polygon();
        let metadata = SpatialMetadata::from_coordinates(&coordinates).unwrap();
        let result = metadata.as_geo_rect();

        assert!(result.is_ok());
        let rect = result.unwrap();
        assert_eq!(rect.min().x, 13.4); // min_lon
        assert_eq!(rect.min().y, 52.5); // min_lat
        assert_eq!(rect.max().x, 13.5); // max_lon
        assert_eq!(rect.max().y, 52.6); // max_lat
    }

    #[test]
    fn test_spatial_metadata_json_serialization() {
        let coordinates = berlin_polygon();
        let metadata = SpatialMetadata::from_coordinates(&coordinates).unwrap();

        let json_result = metadata.to_json_string();
        assert!(json_result.is_ok());
    }

    #[test]
    fn test_spatial_metadata_json_deserialization() {
        let json_str = r#"{"bounding_box":"52.5,13.4,52.6,13.5"}"#;
        let result = SpatialMetadata::from_json_string(json_str);

        assert!(result.is_ok());
        let metadata = result.unwrap();
        assert_eq!(metadata.bounding_box, "52.5,13.4,52.6,13.5");
    }

    #[test]
    fn test_spatial_metadata_json_deserialization_invalid() {
        let json_str = r#"{"invalid": "json"}"#;
        let result = SpatialMetadata::from_json_string(json_str);

        assert!(result.is_err());
    }

    #[test]
    fn test_calculate_bounding_box_string() {
        let coordinates = berlin_polygon();
        let result = calculate_bounding_box_string(&coordinates);

        assert!(result.is_ok());
        let bbox = result.unwrap();
        assert_eq!(bbox, "52.5,13.4,52.6,13.5");
    }

    #[test]
    fn test_calculate_bounding_box_string_empty() {
        let coordinates = vec![];
        let result = calculate_bounding_box_string(&coordinates);

        assert!(result.is_err());
    }

    #[test]
    fn test_calculate_bounding_box_string_single_point() {
        let coordinates = vec![(52.5, 13.4), (52.5, 13.4), (52.5, 13.4), (52.5, 13.4)];
        let result = calculate_bounding_box_string(&coordinates);

        assert!(result.is_ok());
        let bbox = result.unwrap();
        assert_eq!(bbox, "52.5,13.4,52.5,13.4");
    }

    #[test]
    fn test_coordinates_to_geo_polygon() {
        let coordinates = berlin_polygon();
        let result = coordinates_to_geo_polygon(&coordinates);

        assert!(result.is_ok());
        let polygon = result.unwrap();

        // Verify the polygon has the correct number of points
        let exterior = polygon.exterior();
        assert_eq!(exterior.coords().count(), 5); // 4 unique + 1 closing point
    }

    #[test]
    fn test_coordinates_to_geo_polygon_too_few_points() {
        let coordinates = vec![(52.5, 13.4), (52.6, 13.4), (52.5, 13.4)]; // Only 3 points
        let result = coordinates_to_geo_polygon(&coordinates);

        assert!(result.is_err());
    }

    #[test]
    fn test_bounding_box_string_to_geo_rect() {
        let bbox_string = "52.5,13.4,52.6,13.5";
        let result = bounding_box_string_to_geo_rect(bbox_string);

        assert!(result.is_ok());
        let rect = result.unwrap();
        assert_eq!(rect.min().x, 13.4); // min_lon
        assert_eq!(rect.min().y, 52.5); // min_lat
        assert_eq!(rect.max().x, 13.5); // max_lon
        assert_eq!(rect.max().y, 52.6); // max_lat
    }

    #[test]
    fn test_bounding_box_string_to_geo_rect_invalid_format() {
        let bbox_string = "52.5,13.4,52.6"; // Missing one component
        let result = bounding_box_string_to_geo_rect(bbox_string);

        assert!(result.is_err());
    }

    #[test]
    fn test_bounding_box_string_to_geo_rect_invalid_numbers() {
        let bbox_string = "invalid,13.4,52.6,13.5";
        let result = bounding_box_string_to_geo_rect(bbox_string);

        assert!(result.is_err());
    }

    #[test]
    fn test_bounding_boxes_intersect_overlapping() {
        let bbox1 = "52.5,13.4,52.6,13.5"; // Berlin polygon bbox
        let bbox2 = "52.55,13.45,52.65,13.55"; // Overlapping bbox

        let result = bounding_boxes_intersect(bbox1, bbox2);
        assert!(result.is_ok());
        assert!(result.unwrap()); // Should intersect
    }

    #[test]
    fn test_bounding_boxes_intersect_non_overlapping() {
        let bbox1 = "52.5,13.4,52.6,13.5"; // Berlin polygon bbox
        let bbox2 = "50.0,10.0,50.1,10.1"; // Distant bbox

        let result = bounding_boxes_intersect(bbox1, bbox2);
        assert!(result.is_ok());
        assert!(!result.unwrap()); // Should not intersect
    }

    #[test]
    fn test_bounding_boxes_intersect_touching() {
        let bbox1 = "52.5,13.4,52.6,13.5";
        let bbox2 = "52.6,13.5,52.7,13.6"; // Touching at one corner

        let result = bounding_boxes_intersect(bbox1, bbox2);
        assert!(result.is_ok());
        assert!(result.unwrap()); // Should intersect (touching counts as intersection)
    }

    #[test]
    fn test_bounding_boxes_intersect_invalid_bbox() {
        let bbox1 = "52.5,13.4,52.6,13.5";
        let bbox2 = "invalid,bbox,format"; // Invalid format

        let result = bounding_boxes_intersect(bbox1, bbox2);
        assert!(result.is_err());
    }

    #[test]
    fn test_polygons_intersect_overlapping() {
        let polygon1 = berlin_polygon();
        let polygon2 = overlapping_polygon();

        let result = polygons_intersect(&polygon1, &polygon2);
        assert!(result.is_ok());
        assert!(result.unwrap()); // Should intersect
    }

    #[test]
    fn test_polygons_intersect_non_overlapping() {
        let polygon1 = berlin_polygon();
        let polygon2 = distant_polygon();

        let result = polygons_intersect(&polygon1, &polygon2);
        assert!(result.is_ok());
        assert!(!result.unwrap()); // Should not intersect
    }

    #[test]
    fn test_polygons_intersect_identical() {
        let polygon1 = berlin_polygon();
        let polygon2 = berlin_polygon();

        let result = polygons_intersect(&polygon1, &polygon2);
        assert!(result.is_ok());
        assert!(result.unwrap()); // Should intersect (identical polygons)
    }

    #[test]
    fn test_polygons_intersect_invalid_polygon() {
        let polygon1 = berlin_polygon();
        let polygon2 = vec![(52.5, 13.4), (52.6, 13.4)]; // Too few points

        let result = polygons_intersect(&polygon1, &polygon2);
        assert!(result.is_err());
    }

    #[test]
    fn test_triangle_polygon_operations() {
        let triangle = triangle_polygon();

        // Test bounding box calculation
        let bbox_result = calculate_bounding_box_string(&triangle);
        assert!(bbox_result.is_ok());
        let bbox = bbox_result.unwrap();
        assert_eq!(bbox, "0,0,1,1");

        // Test polygon conversion
        let geo_result = coordinates_to_geo_polygon(&triangle);
        assert!(geo_result.is_ok());
    }
}
