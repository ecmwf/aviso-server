//! Wildcard analysis and matching for watch/replay.

use anyhow::{Context, Result};
use aviso_validators::polygon::PolygonHandler;
use geo::{BoundingRect, Intersects};
use std::collections::HashMap;
use tracing::debug;

use crate::notification::topic_codec::{decode_subject, encode_subject, encode_token};

/// Build backend coarse pattern plus decoded full pattern.
pub fn analyze_watch_pattern(watch_topic: &str) -> Result<(String, Vec<String>)> {
    let full_watch_pattern = decode_subject(watch_topic)
        .with_context(|| format!("Failed to decode watch topic pattern '{}'", watch_topic))?;

    let first_wildcard_pos = full_watch_pattern.iter().position(|part| part == "*");

    let backend_subscription_pattern = match first_wildcard_pos {
        Some(pos) if pos > 1 => {
            let specific_parts = &full_watch_pattern[..pos];
            format!("{}.>", encode_subject(specific_parts))
        }
        Some(_) => {
            let base = full_watch_pattern
                .first()
                .cloned()
                .unwrap_or_else(|| "unknown".to_string());
            format!("{}.>", encode_token(&base))
        }
        None => {
            if full_watch_pattern.len() > 1 {
                let without_last = &full_watch_pattern[..full_watch_pattern.len() - 1];
                format!("{}.>", encode_subject(without_last))
            } else {
                format!("{}.>", watch_topic)
            }
        }
    };

    debug!(
        watch_topic = %watch_topic,
        backend_subscription_pattern = %backend_subscription_pattern,
        first_wildcard_pos = ?first_wildcard_pos,
        pattern_parts = full_watch_pattern.len(),
        "Analyzed watch pattern for hybrid filtering"
    );

    Ok((backend_subscription_pattern, full_watch_pattern))
}

/// Position-based wildcard match on decoded topic tokens.
pub fn matches_watch_pattern(notification_topic: &str, watch_pattern: &[String]) -> bool {
    let notification_parts = match decode_subject(notification_topic) {
        Ok(parts) => parts,
        Err(error) => {
            debug!(
                notification_topic = %notification_topic,
                error = %error,
                "Failed to decode notification topic"
            );
            return false;
        }
    };

    if notification_parts.len() != watch_pattern.len() {
        debug!(
            notification_topic = %notification_topic,
            notification_parts = notification_parts.len(),
            pattern_parts = watch_pattern.len(),
            "Topic part count mismatch"
        );
        return false;
    }

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

/// Apply optional polygon filter against metadata/payload geometry.
pub fn matches_notification_filters(
    request: &HashMap<String, String>,
    metadata: Option<&HashMap<String, String>>,
    payload: &str,
) -> bool {
    let Some(request_polygon) = request.get("polygon") else {
        return true;
    };

    debug!(
        "Starting spatial filter check for polygon: {}",
        request_polygon
    );

    let coords_latlon = match PolygonHandler::parse_polygon_coordinates(request_polygon) {
        Ok(coords) => {
            debug!("Parsed request polygon: {} coordinate pairs", coords.len());
            coords
        }
        Err(e) => {
            debug!("Invalid request polygon format: {}", e);
            return false;
        }
    };

    let filter_poly = {
        let coords_lonlat: Vec<(f64, f64)> = coords_latlon
            .iter()
            .map(|(lat, lon)| (*lon, *lat))
            .collect();
        geo::Polygon::new(geo::LineString::from(coords_lonlat), vec![])
    };

    let filter_bbox = match filter_poly.bounding_rect() {
        Some(bbox) => {
            debug!("Request polygon bounding box calculated successfully");
            bbox
        }
        None => {
            debug!("Failed to calculate bounding box for request polygon");
            return false;
        }
    };

    let candidate_bbox = metadata
        .and_then(|m| m.get("spatial_bbox"))
        .and_then(|bbox_str| {
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

    if !candidate_bbox.intersects(&filter_bbox) {
        debug!("Bounding boxes do not intersect - filtering out notification");
        return false;
    }

    debug!("Bounding boxes intersect - proceeding to polygon intersection check");

    let candidate_poly = extract_candidate_polygon(metadata, payload);

    let Some(candidate_poly) = candidate_poly else {
        debug!("No valid polygon geometry found in candidate - filtering out");
        return false;
    };

    let polygons_intersect = candidate_poly.intersects(&filter_poly);

    if polygons_intersect {
        debug!("Polygon intersection successful - notification passes spatial filter");
    } else {
        debug!("Polygons do not intersect - filtering out notification");
    }

    polygons_intersect
}

/// Parse candidate polygon from metadata first, then payload.
fn extract_candidate_polygon(
    metadata: Option<&HashMap<String, String>>,
    payload: &str,
) -> Option<geo::Polygon<f64>> {
    if let Some(candidate_poly) = try_polygon_from_metadata(metadata) {
        return Some(candidate_poly);
    }

    try_polygon_from_payload(payload)
}

/// Parse polygon from metadata `spatial_geometry`.
fn try_polygon_from_metadata(
    metadata: Option<&HashMap<String, String>>,
) -> Option<geo::Polygon<f64>> {
    metadata
        .and_then(|m| m.get("spatial_geometry"))
        .and_then(|geom_str| parse_polygon_geometry_str(geom_str))
}

/// Parse polygon from payload `spatial_geometry`.
fn try_polygon_from_payload(payload: &str) -> Option<geo::Polygon<f64>> {
    let json: serde_json::Value = serde_json::from_str(payload).ok()?;
    let spatial_geometry = json.get("spatial_geometry")?;

    if let Some(geom_obj) = spatial_geometry.as_object() {
        extract_polygon_from_geojson(geom_obj)
    } else {
        spatial_geometry
            .as_str()
            .and_then(parse_polygon_geometry_str)
    }
}

/// Parse coordinate string to geo polygon.
fn parse_polygon_geometry_str(geom_str: &str) -> Option<geo::Polygon<f64>> {
    PolygonHandler::parse_polygon_coordinates(geom_str)
        .ok()
        .map(|coords_latlon| {
            let coords_lonlat: Vec<(f64, f64)> = coords_latlon
                .iter()
                .map(|(lat, lon)| (*lon, *lat))
                .collect();
            geo::Polygon::new(geo::LineString::from(coords_lonlat), vec![])
        })
}

/// Extract polygon from GeoJSON object.
fn extract_polygon_from_geojson(
    geom_obj: &serde_json::Map<String, serde_json::Value>,
) -> Option<geo::Polygon<f64>> {
    let geo_type = geom_obj.get("type")?.as_str()?;
    if geo_type != "Polygon" {
        debug!(
            "Geometry type '{}' is not supported (only Polygon)",
            geo_type
        );
        return None;
    }

    let coords_json = geom_obj.get("coordinates")?;
    let coords_outer = coords_json.as_array()?.first()?;
    let coords_inner = coords_outer.as_array()?;

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

    let coords_lonlat: Vec<(f64, f64)> = coords_latlon
        .iter()
        .map(|(lat, lon)| (*lon, *lat))
        .collect();

    Some(geo::Polygon::new(
        geo::LineString::from(coords_lonlat),
        vec![],
    ))
}

/// Return backend coarse pattern for a watch topic.
pub fn generate_backend_subscription_pattern(watch_topic: &str) -> String {
    analyze_watch_pattern(watch_topic)
        .map(|(backend_pattern, _)| backend_pattern)
        .unwrap_or_else(|_| format!("{}.>", watch_topic))
}

/// Build reusable matcher closure for a decoded watch pattern.
pub fn create_pattern_matcher(watch_pattern: Vec<String>) -> impl Fn(&str) -> bool {
    move |notification_topic: &str| matches_watch_pattern(notification_topic, &watch_pattern)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_analyze_watch_pattern_early_wildcard() {
        let (backend_pattern, app_pattern) =
            analyze_watch_pattern("diss.FOO.%2A.od.%2A.%2A.%2A.%2A.%2A.%2A").unwrap();
        assert_eq!(backend_pattern, "diss.FOO.>");
        assert_eq!(
            app_pattern,
            vec!["diss", "FOO", "*", "od", "*", "*", "*", "*", "*", "*"]
        );
    }

    #[test]
    fn test_analyze_watch_pattern_late_wildcard() {
        let (backend_pattern, app_pattern) =
            analyze_watch_pattern("diss.FOO.E1.od.%2A.%2A.%2A.%2A.%2A.%2A").unwrap();
        assert_eq!(backend_pattern, "diss.FOO.E1.od.>");
        assert_eq!(
            app_pattern,
            vec!["diss", "FOO", "E1", "od", "*", "*", "*", "*", "*", "*"]
        );
    }

    #[test]
    fn test_analyze_watch_pattern_immediate_wildcard() {
        let (backend_pattern, app_pattern) =
            analyze_watch_pattern("diss.%2A.%2A.%2A.%2A.%2A.%2A.%2A.%2A.%2A").unwrap();
        assert_eq!(backend_pattern, "diss.>");
        assert_eq!(
            app_pattern,
            vec!["diss", "*", "*", "*", "*", "*", "*", "*", "*", "*"]
        );
    }

    #[test]
    fn test_analyze_watch_pattern_no_wildcards() {
        let (backend_pattern, app_pattern) =
            analyze_watch_pattern("diss.FOO.E1.od.0001.g.20260706.0000.enfo.1").unwrap();
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
    fn test_matches_watch_pattern_with_encoded_notification_tokens() {
        let pattern = vec![
            "diss".to_string(),
            "FOO".to_string(),
            "1.45".to_string(),
            "p%q".to_string(),
        ];
        assert!(matches_watch_pattern("diss.FOO.1%2E45.p%25q", &pattern));
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
