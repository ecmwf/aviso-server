use anyhow::{Result, bail};
use tracing::debug;

/// Polygon coordinate validator
///
/// Validates polygon coordinate strings in the format "(lat1,lon1,lat2,lon2,lat1,lon1)"
/// and ensures proper polygon geometry (closed, minimum vertices, valid coordinates).
pub struct PolygonHandler;

impl PolygonHandler {
    /// Validates and canonicalizes polygon coordinate strings
    ///
    /// # Arguments
    /// * `value` - The polygon coordinate string to validate
    /// * `field_name` - Name of the field being validated (for error messages)
    ///
    /// # Returns
    /// * `Ok(String)` - The validated coordinate string
    /// * `Err(anyhow::Error)` - Validation failed with detailed error
    pub fn validate_and_canonicalize(value: &str, field_name: &str) -> Result<String> {
        debug!("Validating polygon field '{}' with value: {}", field_name, value);

        // Parse the coordinate string
        let coordinates = Self::parse_polygon_coordinates(value)?;
        debug!("Parsed {} coordinate pairs for field '{}'", coordinates.len(), field_name);

        // Validate the polygon
        Self::validate_polygon_geometry(&coordinates)?;
        debug!("Polygon geometry validation passed for field '{}'", field_name);

        // Return the original validated string
        // (JSON conversion will happen elsewhere when building the payload)
        Ok(value.to_string())
    }

    /// Parses polygon coordinate string into coordinate pairs
    fn parse_polygon_coordinates(coord_string: &str) -> Result<Vec<(f64, f64)>> {
        // Remove parentheses and trim whitespace
        let trimmed = coord_string
            .trim()
            .trim_start_matches('(')
            .trim_end_matches(')')
            .trim();

        if trimmed.is_empty() {
            bail!("Empty polygon coordinate string");
        }

        // Split by comma and parse coordinate pairs
        let coord_parts: Vec<&str> = trimmed.split(',').collect();

        if coord_parts.len() % 2 != 0 {
            bail!("Polygon coordinates must be in pairs (lat,lon)");
        }

        let mut coordinates = Vec::new();
        let mut iter = coord_parts.iter();

        while let Some(lat_str) = iter.next() {
            let lon_str = iter.next().unwrap(); // Safe because we checked length above

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
}
