//! Wildcard pattern analysis and matching for watch endpoint subscriptions
//!
//! This module provides intelligent wildcard pattern analysis that optimizes
//! backend subscriptions while maintaining flexible application-level filtering.
//! It implements a hybrid approach where the backend handles coarse filtering
//! and the application handles fine-grained pattern matching.

use crate::notification::validators::PolygonHandler;
use geo::{BoundingRect, Intersects};
use std::collections::HashMap;
use tracing::debug;

/// Analyze a watch pattern to determine optimal backend subscription and application filter
///
/// This function implements the hybrid wildcard strategy:
/// 1. Find the first wildcard position in the watch pattern
/// 2. Generate the most specific backend pattern possible (up to first wildcard)
/// 3. Return the full watch pattern for application-level filtering
///
/// # Arguments
/// * `watch_topic` - The topic pattern from the watch request (e.g., "diss.FOO.*.od.*.*.*.*.*.*")
///
/// # Returns
/// * `(String, Vec<String>)` - (backend subscription pattern, full watch pattern as Vec)
pub fn analyze_watch_pattern(watch_topic: &str) -> (String, Vec<String>) {
    let parts: Vec<&str> = watch_topic.split('.').collect();

    // Convert to owned strings for the full pattern
    let full_watch_pattern: Vec<String> = parts.iter().map(|s| s.to_string()).collect();

    // Find the first wildcard position
    let first_wildcard_pos = parts.iter().position(|&part| part == "*");

    let backend_subscription_pattern = match first_wildcard_pos {
        Some(pos) if pos > 1 => {
            // Use JetStream '>' wildcard for everything after first wildcard position
            let specific_parts = &parts[..pos];
            format!("{}.>", specific_parts.join(".")) // Use > instead of .*
        }
        Some(_) => {
            // Wildcard at position 0 or 1, use broad pattern with just the base
            let base = parts.first().map_or("unknown", |v| *v);
            format!("{}.>", base) // Use > instead of .*
        }
        None => {
            // No wildcards present, use specific pattern with > for potential sub-topics
            if parts.len() > 1 {
                let without_last = &parts[..parts.len() - 1];
                format!("{}.>", without_last.join(".")) // Use > instead of .*
            } else {
                // Single part topic, use > wildcard
                format!("{}.>", watch_topic) // Use > instead of .*
            }
        }
    };

    debug!(
        watch_topic = %watch_topic,
        backend_subscription_pattern = %backend_subscription_pattern,
        first_wildcard_pos = ?first_wildcard_pos,
        pattern_parts = parts.len(),
        "Analyzed watch pattern for hybrid filtering"
    );

    (backend_subscription_pattern, full_watch_pattern)
}

/// Check if a notification topic matches a watch pattern
///
/// This function performs position-based pattern matching where:
/// - Non-wildcard parts must match exactly
/// - Wildcard parts ("*") match any value
/// - Both topic and pattern must have the same number of parts
///
/// # Arguments
/// * `notification_topic` - The actual topic from a notification (e.g., "diss.FOO.E1.od.0001.g.20260706.0000.enfo.1")
/// * `watch_pattern` - The watch pattern as a Vec of parts (e.g., ["diss", "FOO", "*", "od", "*", "*", "*", "*", "*", "*"])
///
/// # Returns
/// * `bool` - true if the notification topic matches the watch pattern
pub fn matches_watch_pattern(notification_topic: &str, watch_pattern: &[String]) -> bool {
    let notification_parts: Vec<&str> = notification_topic.split('.').collect();

    // Must have the same number of parts
    if notification_parts.len() != watch_pattern.len() {
        debug!(
            notification_topic = %notification_topic,
            notification_parts = notification_parts.len(),
            pattern_parts = watch_pattern.len(),
            "Topic part count mismatch"
        );
        return false;
    }

    // Check each position
    for (i, (notif_part, pattern_part)) in notification_parts
        .iter()
        .zip(watch_pattern.iter())
        .enumerate()
    {
        if pattern_part != "*" && pattern_part != notif_part {
            debug!(
                notification_topic = %notification_topic,
                position = i,
                notification_part = %notif_part,
                pattern_part = %pattern_part,
                "Pattern mismatch at position"
            );
            return false;
        }
    }

    debug!(
        notification_topic = %notification_topic,
        "Topic matches watch pattern"
    );

    true
}

/// Determines if a notification matches the given filters, supporting spatial (polygon) filtering.
///
/// This function applies spatial filtering when a polygon is specified in the request.
/// It performs efficient two-stage filtering:
/// 1. Fast bounding box intersection check to eliminate obvious non-matches
/// 2. Precise polygon intersection for candidates that pass the bbox test
///
/// # Arguments
/// * `request` - Request parameters including optional polygon filter
/// * `metadata` - Optional notification metadata containing spatial bounding box
/// * `payload` - Notification payload as JSON string (must contain HashMap for spatial data)
///
/// # Returns
/// * `bool` - true if notification matches all filters, false otherwise
pub fn matches_notification_filters(
    request: &HashMap<String, String>,
    metadata: Option<&HashMap<String, String>>,
    payload: &str,
) -> bool {
    // Only apply spatial filtering if a polygon is specified in the request
    let Some(request_polygon) = request.get("polygon") else {
        // No spatial filtering requested - notification matches
        return true;
    };

    debug!(
        "Starting spatial filter check for polygon: {}",
        request_polygon
    );

    // Parse and validate the request polygon coordinates
    let coords_latlon = match PolygonHandler::parse_polygon_coordinates(request_polygon) {
        Ok(coords) => {
            debug!("Parsed request polygon: {} coordinate pairs", coords.len());
            coords
        }
        Err(e) => {
            debug!("Invalid request polygon format: {}", e);
            return false; // Malformed request polygon, treat as non-match
        }
    };

    // Build geo::Polygon from coordinates
    let filter_poly = {
        let coords_lonlat: Vec<(f64, f64)> = coords_latlon
            .iter()
            .map(|(lat, lon)| (*lon, *lat))
            .collect();
        geo::Polygon::new(geo::LineString::from(coords_lonlat), vec![])
    };

    // Calculate bounding box for the filter polygon
    let filter_bbox = match filter_poly.bounding_rect() {
        Some(bbox) => {
            debug!("Request polygon bounding box calculated successfully");
            bbox
        }
        None => {
            debug!("Failed to calculate bounding box for request polygon");
            return false; // Degenerate polygon
        }
    };

    // Extract candidate's bounding box from metadata
    let candidate_bbox = metadata
        .and_then(|m| m.get("spatial_bbox"))
        .and_then(|bbox_str| {
            // Parse bbox string: "min_lat,min_lon,max_lat,max_lon"
            let coords: Vec<f64> = bbox_str
                .split(',')
                .map(|part| part.trim().parse().ok())
                .collect::<Option<Vec<_>>>()?;

            if coords.len() != 4 {
                debug!(
                    "Malformed candidate bbox: expected 4 values, got {}",
                    coords.len()
                );
                return None;
            }

            let (min_lat, min_lon, max_lat, max_lon) = (coords[0], coords[1], coords[2], coords[3]);

            // Create geo::Rect (expects lon,lat coordinates)
            Some(geo::Rect::new(
                geo::Coord {
                    x: min_lon,
                    y: min_lat,
                },
                geo::Coord {
                    x: max_lon,
                    y: max_lat,
                },
            ))
        });

    let Some(candidate_bbox) = candidate_bbox else {
        debug!("No valid spatial_bbox found in candidate metadata");
        return false;
    };

    // Fast filtering: check if bounding boxes intersect
    if !candidate_bbox.intersects(&filter_bbox) {
        debug!("Bounding boxes do not intersect - filtering out notification");
        return false;
    }

    debug!("Bounding boxes intersect - proceeding to polygon intersection check");

    // Extract candidate polygon geometry from metadata or payload
    let candidate_poly = extract_candidate_polygon(metadata, payload);

    let Some(candidate_poly) = candidate_poly else {
        debug!("No valid polygon geometry found in candidate - filtering out");
        return false;
    };

    // Perform precise polygon intersection check
    let polygons_intersect = candidate_poly.intersects(&filter_poly);

    if polygons_intersect {
        debug!("Polygon intersection successful - notification passes spatial filter");
    } else {
        debug!("Polygons do not intersect - filtering out notification");
    }

    polygons_intersect
}

/// Extract candidate polygon from metadata or payload
///
/// Tries to find polygon geometry in this order:
/// 1. spatial_geometry field in metadata (as coordinate string)
/// 2. spatial_geometry field in payload (as GeoJSON object)
///
/// # Arguments
/// * `metadata` - Optional notification metadata
/// * `payload` - Notification payload as JSON string
///
/// # Returns
/// * `Option<geo::Polygon>` - Parsed polygon or None if not found/invalid
fn extract_candidate_polygon(
    metadata: Option<&HashMap<String, String>>,
    payload: &str,
) -> Option<geo::Polygon<f64>> {
    // First, try to get polygon from metadata
    if let Some(candidate_poly) = try_polygon_from_metadata(metadata) {
        return Some(candidate_poly);
    }

    // Fallback: try to extract from payload (HashMap only)
    try_polygon_from_payload(payload)
}

/// Try to extract polygon from metadata spatial_geometry field
fn try_polygon_from_metadata(
    metadata: Option<&HashMap<String, String>>,
) -> Option<geo::Polygon<f64>> {
    metadata
        .and_then(|m| m.get("spatial_geometry"))
        .and_then(|geom_str| {
            PolygonHandler::parse_polygon_coordinates(geom_str)
                .ok()
                .map(|coords_latlon| {
                    // Convert (lat,lon) to (lon,lat) for geo crate
                    let coords_lonlat: Vec<(f64, f64)> = coords_latlon
                        .iter()
                        .map(|(lat, lon)| (*lon, *lat))
                        .collect();
                    geo::Polygon::new(geo::LineString::from(coords_lonlat), vec![])
                })
        })
}

/// Try to extract polygon from payload spatial_geometry field
///
/// Since your code only works with HashMap payloads, this function assumes
/// the payload contains a JSON object with a spatial_geometry field.
fn try_polygon_from_payload(payload: &str) -> Option<geo::Polygon<f64>> {
    // Parse payload as JSON object (HashMap only)
    let json: serde_json::Value = serde_json::from_str(payload).ok()?;
    let spatial_geometry = json.get("spatial_geometry")?;

    // Handle GeoJSON-style geometry object
    if let Some(geom_obj) = spatial_geometry.as_object() {
        extract_polygon_from_geojson(geom_obj)
    } else {
        // Handle geometry as coordinate string
        spatial_geometry.as_str().and_then(|geom_str| {
            PolygonHandler::parse_polygon_coordinates(geom_str)
                .ok()
                .map(|coords_latlon| {
                    let coords_lonlat: Vec<(f64, f64)> = coords_latlon
                        .iter()
                        .map(|(lat, lon)| (*lon, *lat))
                        .collect();
                    geo::Polygon::new(geo::LineString::from(coords_lonlat), vec![])
                })
        })
    }
}

/// Extract polygon from GeoJSON-style geometry object
fn extract_polygon_from_geojson(
    geom_obj: &serde_json::Map<String, serde_json::Value>,
) -> Option<geo::Polygon<f64>> {
    // Check if this is a Polygon type
    let geo_type = geom_obj.get("type")?.as_str()?;
    if geo_type != "Polygon" {
        debug!(
            "Geometry type '{}' is not supported (only Polygon)",
            geo_type
        );
        return None;
    }

    // Extract coordinates array
    let coords_json = geom_obj.get("coordinates")?;
    let coords_outer = coords_json.as_array()?.first()?; // Get outer ring
    let coords_inner = coords_outer.as_array()?;

    // Parse coordinate pairs (assuming lat,lon order in your GeoJSON)
    let coords_latlon: Vec<(f64, f64)> = coords_inner
        .iter()
        .filter_map(|pair| {
            let lat = pair.get(0)?.as_f64()?;
            let lon = pair.get(1)?.as_f64()?;
            Some((lat, lon))
        })
        .collect();

    if coords_latlon.len() < 3 {
        debug!(
            "Insufficient coordinates for polygon: {}",
            coords_latlon.len()
        );
        return None;
    }

    // Convert to geo::Polygon (swap to lon,lat for geo crate)
    let coords_lonlat: Vec<(f64, f64)> = coords_latlon
        .iter()
        .map(|(lat, lon)| (*lon, *lat))
        .collect();

    Some(geo::Polygon::new(
        geo::LineString::from(coords_lonlat),
        vec![],
    ))
}

/// Generate backend-compatible wildcard pattern from a topic pattern
///
/// This is a convenience function that extracts just the backend subscription pattern
/// from the analysis, useful when you only need the subscription pattern.
///
/// # Arguments
/// * `watch_topic` - The topic pattern from the watch request
///
/// # Returns
/// * `String` - The backend subscription pattern
pub fn generate_backend_subscription_pattern(watch_topic: &str) -> String {
    let (backend_pattern, _) = analyze_watch_pattern(watch_topic);
    backend_pattern
}

/// Create a pattern matcher function for a specific watch pattern
///
/// This function returns a closure that can be used to efficiently test
/// multiple notification topics against the same watch pattern.
///
/// # Arguments
/// * `watch_pattern` - The watch pattern as a Vec of parts
///
/// # Returns
/// * `impl Fn(&str) -> bool` - A closure that tests notification topics
pub fn create_pattern_matcher(watch_pattern: Vec<String>) -> impl Fn(&str) -> bool {
    move |notification_topic: &str| matches_watch_pattern(notification_topic, &watch_pattern)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_analyze_watch_pattern_early_wildcard() {
        let (backend_pattern, app_pattern) = analyze_watch_pattern("diss.FOO.*.od.*.*.*.*.*.*");
        assert_eq!(backend_pattern, "diss.FOO.>");
        assert_eq!(
            app_pattern,
            vec!["diss", "FOO", "*", "od", "*", "*", "*", "*", "*", "*"]
        );
    }

    #[test]
    fn test_analyze_watch_pattern_late_wildcard() {
        let (backend_pattern, app_pattern) = analyze_watch_pattern("diss.FOO.E1.od.*.*.*.*.*.*");
        assert_eq!(backend_pattern, "diss.FOO.E1.od.>");
        assert_eq!(
            app_pattern,
            vec!["diss", "FOO", "E1", "od", "*", "*", "*", "*", "*", "*"]
        );
    }

    #[test]
    fn test_analyze_watch_pattern_immediate_wildcard() {
        let (backend_pattern, app_pattern) = analyze_watch_pattern("diss.*.*.*.*.*.*.*.*.*");
        assert_eq!(backend_pattern, "diss.>");
        assert_eq!(
            app_pattern,
            vec!["diss", "*", "*", "*", "*", "*", "*", "*", "*", "*"]
        );
    }

    #[test]
    fn test_analyze_watch_pattern_no_wildcards() {
        let (backend_pattern, app_pattern) =
            analyze_watch_pattern("diss.FOO.E1.od.0001.g.20260706.0000.enfo.1");
        assert_eq!(
            backend_pattern,
            "diss.FOO.E1.od.0001.g.20260706.0000.enfo.>"
        );
        assert_eq!(
            app_pattern,
            vec![
                "diss", "FOO", "E1", "od", "0001", "g", "20260706", "0000", "enfo", "1"
            ]
        );
    }

    #[test]
    fn test_matches_watch_pattern_exact_match() {
        let pattern = vec!["diss".to_string(), "FOO".to_string(), "E1".to_string()];
        assert!(matches_watch_pattern("diss.FOO.E1", &pattern));
    }

    #[test]
    fn test_matches_watch_pattern_with_wildcards() {
        let pattern = vec![
            "diss".to_string(),
            "FOO".to_string(),
            "*".to_string(),
            "od".to_string(),
        ];
        assert!(matches_watch_pattern("diss.FOO.E1.od", &pattern));
        assert!(matches_watch_pattern("diss.FOO.E2.od", &pattern));
        assert!(!matches_watch_pattern("diss.FOO.E1.mars", &pattern));
    }

    #[test]
    fn test_matches_watch_pattern_length_mismatch() {
        let pattern = vec!["diss".to_string(), "FOO".to_string()];
        assert!(!matches_watch_pattern("diss.FOO.E1", &pattern));
    }

    #[test]
    fn test_matches_watch_pattern_complex() {
        let pattern = vec![
            "diss".to_string(),
            "FOO".to_string(),
            "*".to_string(),
            "od".to_string(),
            "*".to_string(),
            "*".to_string(),
            "*".to_string(),
            "*".to_string(),
            "*".to_string(),
            "*".to_string(),
        ];

        assert!(matches_watch_pattern(
            "diss.FOO.E1.od.0001.g.20260706.0000.enfo.1",
            &pattern
        ));
        assert!(matches_watch_pattern(
            "diss.FOO.E2.od.0002.g.20260707.1200.enfo.2",
            &pattern
        ));
        assert!(!matches_watch_pattern(
            "diss.BAR.E1.od.0001.g.20260706.0000.enfo.1",
            &pattern
        ));
        assert!(!matches_watch_pattern(
            "mars.FOO.E1.od.0001.g.20260706.0000.enfo.1",
            &pattern
        ));
    }

    #[test]
    fn test_create_pattern_matcher() {
        let pattern = vec!["diss".to_string(), "*".to_string(), "E1".to_string()];
        let matcher = create_pattern_matcher(pattern);

        assert!(matcher("diss.FOO.E1"));
        assert!(matcher("diss.BAR.E1"));
        assert!(!matcher("diss.FOO.E2"));
        assert!(!matcher("mars.FOO.E1"));
    }
}
