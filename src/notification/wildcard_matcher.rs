//! Wildcard analysis and matching for watch/replay.

use anyhow::{Context, Result};
use aviso_validators::{PointHandler, polygon::PolygonHandler};
use geo::{BoundingRect, Contains, Intersects, Point};
use std::collections::HashMap;
use tracing::debug;

use crate::configuration::{EventSchema, Settings};
use crate::notification::IdentifierConstraint;
use crate::notification::topic_codec::{decode_subject, encode_subject, encode_token};
use crate::notification::topic_parser::topic_to_request;

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
    notification_topic: &str,
    request: &HashMap<String, String>,
    constraints: &HashMap<String, IdentifierConstraint>,
    metadata: Option<&HashMap<String, String>>,
    payload: &str,
) -> bool {
    if !constraints.is_empty() && !matches_identifier_constraints(notification_topic, constraints) {
        return false;
    }

    let has_explicit_polygon = request.get("polygon").is_some_and(|polygon| polygon != "*");

    if has_explicit_polygon && request.contains_key("point") {
        debug!("Both polygon and point are set in request filter; rejecting");
        return false;
    }

    if let Some(request_point) = request.get("point") {
        return matches_point_spatial_filter(request_point, metadata, payload);
    }

    let Some(request_polygon) = request.get("polygon") else {
        return true;
    };

    if request_polygon == "*" {
        debug!("Polygon filter is wildcard; skipping polygon intersection filtering");
        return true;
    }

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

    let candidate_bbox = extract_candidate_bbox(metadata);

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

fn matches_identifier_constraints(
    notification_topic: &str,
    constraints: &HashMap<String, IdentifierConstraint>,
) -> bool {
    let Some(schema_map) = Settings::get_global_notification_schema().as_ref() else {
        debug!(
            notification_topic = %notification_topic,
            "Global schema map unavailable for constraint evaluation"
        );
        return false;
    };

    let decoded_topic = match decode_subject(notification_topic) {
        Ok(decoded_topic) => decoded_topic,
        Err(error) => {
            debug!(
                notification_topic = %notification_topic,
                error = %error,
                "Failed to decode topic for constraint evaluation"
            );
            return false;
        }
    };

    let event_type = match resolve_event_type_from_topic_base(&decoded_topic, schema_map) {
        Some(event_type) => event_type,
        None => {
            debug!(
                notification_topic = %notification_topic,
                "Failed to resolve schema event type from topic base for constraint evaluation"
            );
            return false;
        }
    };

    // Fast path: evaluate constraints directly from decoded topic tokens and
    // schema key_order, without reconstructing the full identifier map.
    if let Some(matched) = matches_identifier_constraints_fast_path(
        &decoded_topic,
        event_type,
        constraints,
        schema_map,
    ) {
        return matched;
    }

    // Fallback keeps previous behavior when fast-path prerequisites are not met.
    let identifier = match topic_to_request(notification_topic, event_type) {
        Ok(identifier) => identifier,
        Err(error) => {
            debug!(
                notification_topic = %notification_topic,
                event_type = %event_type,
                error = %error,
                "Failed to parse topic identifier for constraint evaluation"
            );
            return false;
        }
    };

    for (field, constraint) in constraints {
        let Some(raw_value) = identifier.get(field) else {
            debug!(
                field = %field,
                notification_topic = %notification_topic,
                "Constraint field missing in notification identifier"
            );
            return false;
        };

        if !matches_constraint_value(raw_value, constraint) {
            return false;
        }
    }

    true
}

fn matches_identifier_constraints_fast_path(
    decoded_topic: &[String],
    event_type: &str,
    constraints: &HashMap<String, IdentifierConstraint>,
    schema_map: &HashMap<String, EventSchema>,
) -> Option<bool> {
    let event_schema = schema_map.get(event_type)?;
    let topic_config = event_schema.topic.as_ref()?;

    if decoded_topic.first() != Some(&topic_config.base) {
        return Some(false);
    }

    for (field, constraint) in constraints {
        let Some(field_position) = topic_config.key_order.iter().position(|key| key == field)
        else {
            return Some(false);
        };

        let token_index = field_position + 1;
        let Some(raw_value) = decoded_topic.get(token_index) else {
            return Some(false);
        };

        // Keep parity with topic_to_request behavior: wildcard/empty means omitted key.
        if raw_value.is_empty() || raw_value == "*" {
            return Some(false);
        }

        if !matches_constraint_value(raw_value, constraint) {
            return Some(false);
        }
    }

    Some(true)
}

fn matches_constraint_value(raw_value: &str, constraint: &IdentifierConstraint) -> bool {
    match constraint {
        IdentifierConstraint::Int(constraint) => raw_value
            .parse::<i64>()
            .ok()
            .is_some_and(|value| constraint.matches(value)),
        IdentifierConstraint::Enum(constraint) => constraint.matches(raw_value),
        IdentifierConstraint::Float(constraint) => raw_value
            .parse::<f64>()
            .ok()
            .is_some_and(|value| constraint.matches(value)),
    }
}

fn resolve_event_type_from_topic_base<'a>(
    decoded_topic: &[String],
    schema_map: &'a HashMap<String, EventSchema>,
) -> Option<&'a str> {
    let topic_base = decoded_topic.first()?;

    // Topic base can differ from schema key (e.g. base "diss", key
    // "dissemination"), so resolution must match by schema.topic.base.
    schema_map.iter().find_map(|(event_type, schema)| {
        schema
            .topic
            .as_ref()
            .filter(|topic| topic.base == *topic_base)
            .map(|_| event_type.as_str())
    })
}

fn matches_point_spatial_filter(
    request_point: &str,
    metadata: Option<&HashMap<String, String>>,
    payload: &str,
) -> bool {
    let point = match PointHandler::parse_point_coordinates(request_point) {
        Ok((lat, lon)) => Point::new(lon, lat),
        Err(_) => {
            debug!("Invalid point filter format");
            return false;
        }
    };

    if let Some(candidate_bbox) = extract_candidate_bbox(metadata) {
        if !bbox_contains_point(&candidate_bbox, point.x(), point.y()) {
            debug!("Point is outside candidate bounding box");
            return false;
        }
    } else {
        debug!("No valid spatial_bbox found in candidate metadata; using geometry fallback");
    }

    let Some(candidate_poly) = extract_candidate_polygon(metadata, payload) else {
        debug!("No valid polygon geometry found in candidate - filtering out");
        return false;
    };

    let contains_point = candidate_poly.contains(&point);
    if contains_point {
        debug!("Point is inside candidate polygon");
    } else {
        debug!("Point is outside candidate polygon");
    }
    contains_point
}

fn bbox_contains_point(bbox: &geo::Rect<f64>, lon: f64, lat: f64) -> bool {
    lon >= bbox.min().x && lon <= bbox.max().x && lat >= bbox.min().y && lat <= bbox.max().y
}

fn extract_candidate_bbox(metadata: Option<&HashMap<String, String>>) -> Option<geo::Rect<f64>> {
    metadata
        .and_then(|m| m.get("spatial_bbox"))
        .and_then(|bbox| parse_bbox(bbox))
}

fn parse_bbox(bbox_str: &str) -> Option<geo::Rect<f64>> {
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
    use crate::configuration::{
        ApplicationSettings, AuthSettings, EventSchema, NotificationBackendSettings, PayloadConfig,
        Settings, TopicConfig, WatchEndpointSettings,
    };
    use aviso_validators::{EnumConstraint, NumericConstraint};
    use std::collections::HashMap;
    use std::sync::Once;

    static GLOBAL_SCHEMA_INIT: Once = Once::new();

    fn ensure_global_schema_for_constraint_tests() {
        GLOBAL_SCHEMA_INIT.call_once(|| {
            let mars_schema = EventSchema {
                payload: Some(PayloadConfig { required: false }),
                topic: Some(TopicConfig {
                    base: "mars".to_string(),
                    key_order: vec![
                        "class".to_string(),
                        "expver".to_string(),
                        "domain".to_string(),
                        "date".to_string(),
                        "time".to_string(),
                        "stream".to_string(),
                        "step".to_string(),
                    ],
                }),
                endpoint: None,
                identifier: HashMap::new(),
                storage_policy: None,
                auth: None,
            };
            let extreme_schema = EventSchema {
                payload: Some(PayloadConfig { required: false }),
                topic: Some(TopicConfig {
                    base: "extreme".to_string(),
                    key_order: vec!["severity".to_string()],
                }),
                endpoint: None,
                identifier: HashMap::new(),
                storage_policy: None,
                auth: None,
            };
            let dissemination_schema = EventSchema {
                payload: Some(PayloadConfig { required: false }),
                topic: Some(TopicConfig {
                    base: "diss".to_string(),
                    key_order: vec!["destination".to_string(), "step".to_string()],
                }),
                endpoint: None,
                identifier: HashMap::new(),
                storage_policy: None,
                auth: None,
            };

            let settings = Settings {
                application: ApplicationSettings {
                    host: "127.0.0.1".to_string(),
                    port: 8000,
                    base_url: "localhost:8000".to_string(),
                    static_files_path: "./src/static".to_string(),
                },
                notification_backend: NotificationBackendSettings {
                    kind: "in_memory".to_string(),
                    in_memory: None,
                    jetstream: None,
                },
                logging: None,
                notification_schema: Some(HashMap::from([
                    ("mars".to_string(), mars_schema),
                    ("extreme".to_string(), extreme_schema),
                    ("dissemination".to_string(), dissemination_schema),
                ])),
                watch_endpoint: WatchEndpointSettings::default(),
                auth: AuthSettings::default(),
            };

            settings.init_global_config();
        });
    }

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

    #[test]
    fn test_matches_notification_filters_with_point_inside_polygon() {
        let mut request = HashMap::new();
        request.insert("point".to_string(), "52.55,13.5".to_string());
        let constraints = HashMap::new();

        let mut metadata = HashMap::new();
        metadata.insert(
            "spatial_bbox".to_string(),
            "52.4,13.4,52.6,13.6".to_string(),
        );
        metadata.insert(
            "spatial_geometry".to_string(),
            "(52.5,13.4,52.6,13.5,52.5,13.6,52.4,13.5,52.5,13.4)".to_string(),
        );

        assert!(matches_notification_filters(
            "polygon.20250706.1200",
            &request,
            &constraints,
            Some(&metadata),
            "{}"
        ));
    }

    #[test]
    fn test_matches_notification_filters_with_point_outside_polygon() {
        let mut request = HashMap::new();
        request.insert("point".to_string(), "0,0".to_string());
        let constraints = HashMap::new();

        let mut metadata = HashMap::new();
        metadata.insert(
            "spatial_bbox".to_string(),
            "52.4,13.4,52.6,13.6".to_string(),
        );
        metadata.insert(
            "spatial_geometry".to_string(),
            "(52.5,13.4,52.6,13.5,52.5,13.6,52.4,13.5,52.5,13.4)".to_string(),
        );

        assert!(!matches_notification_filters(
            "polygon.20250706.1200",
            &request,
            &constraints,
            Some(&metadata),
            "{}"
        ));
    }

    #[test]
    fn test_matches_notification_filters_with_wildcard_polygon_and_point() {
        let mut request = HashMap::new();
        request.insert("polygon".to_string(), "*".to_string());
        request.insert("point".to_string(), "52.55,13.5".to_string());
        let constraints = HashMap::new();

        let mut metadata = HashMap::new();
        metadata.insert(
            "spatial_bbox".to_string(),
            "52.4,13.4,52.6,13.6".to_string(),
        );
        metadata.insert(
            "spatial_geometry".to_string(),
            "(52.5,13.4,52.6,13.5,52.5,13.6,52.4,13.5,52.5,13.4)".to_string(),
        );

        assert!(matches_notification_filters(
            "polygon.20250706.1200",
            &request,
            &constraints,
            Some(&metadata),
            "{}"
        ));
    }

    #[test]
    fn test_matches_notification_filters_with_point_and_payload_geometry() {
        let mut request = HashMap::new();
        request.insert("point".to_string(), "52.55,13.5".to_string());
        let constraints = HashMap::new();

        let mut metadata = HashMap::new();
        metadata.insert(
            "spatial_bbox".to_string(),
            "52.4,13.4,52.6,13.6".to_string(),
        );

        let payload = serde_json::json!({
            "note": "inside",
            "spatial_geometry": {
                "type": "Polygon",
                "coordinates": [[
                    [52.5, 13.4],
                    [52.6, 13.5],
                    [52.5, 13.6],
                    [52.4, 13.5],
                    [52.5, 13.4]
                ]]
            }
        })
        .to_string();

        assert!(matches_notification_filters(
            "polygon.20250706.1200",
            &request,
            &constraints,
            Some(&metadata),
            &payload
        ));
    }

    #[test]
    fn test_matches_notification_filters_with_point_and_payload_geometry_without_bbox() {
        let mut request = HashMap::new();
        request.insert("point".to_string(), "52.55,13.5".to_string());
        let constraints = HashMap::new();

        let payload = serde_json::json!({
            "note": "inside",
            "spatial_geometry": {
                "type": "Polygon",
                "coordinates": [[
                    [52.5, 13.4],
                    [52.6, 13.5],
                    [52.5, 13.6],
                    [52.4, 13.5],
                    [52.5, 13.4]
                ]]
            }
        })
        .to_string();

        assert!(matches_notification_filters(
            "polygon.20250706.1200",
            &request,
            &constraints,
            None,
            &payload
        ));
    }

    #[test]
    fn test_matches_notification_filters_with_wildcard_polygon_without_point() {
        let mut request = HashMap::new();
        request.insert("polygon".to_string(), "*".to_string());
        let constraints = HashMap::new();

        assert!(matches_notification_filters(
            "polygon.20250706.1200",
            &request,
            &constraints,
            None,
            ""
        ));
    }

    #[test]
    fn test_matches_notification_filters_with_int_constraint() {
        ensure_global_schema_for_constraint_tests();
        let request = HashMap::new();
        let mut constraints = HashMap::new();
        constraints.insert(
            "step".to_string(),
            IdentifierConstraint::Int(NumericConstraint::Gte(4)),
        );

        assert!(matches_notification_filters(
            "mars.od.0001.g.20250706.1200.enfo.5",
            &request,
            &constraints,
            None,
            ""
        ));
        assert!(!matches_notification_filters(
            "mars.od.0001.g.20250706.1200.enfo.2",
            &request,
            &constraints,
            None,
            ""
        ));
    }

    #[test]
    fn test_matches_notification_filters_with_enum_constraint() {
        ensure_global_schema_for_constraint_tests();
        let request = HashMap::new();
        let mut constraints = HashMap::new();
        constraints.insert(
            "domain".to_string(),
            IdentifierConstraint::Enum(EnumConstraint::In(vec!["g".to_string(), "a".to_string()])),
        );

        assert!(matches_notification_filters(
            "mars.od.0001.g.20250706.1200.enfo.1",
            &request,
            &constraints,
            None,
            ""
        ));
        assert!(!matches_notification_filters(
            "mars.od.0001.z.20250706.1200.enfo.1",
            &request,
            &constraints,
            None,
            ""
        ));
    }

    #[test]
    fn test_matches_notification_filters_with_float_constraint() {
        ensure_global_schema_for_constraint_tests();
        let request = HashMap::new();
        let mut constraints = HashMap::new();
        constraints.insert(
            "severity".to_string(),
            IdentifierConstraint::Float(NumericConstraint::Gt(3.5)),
        );

        // Wire topics percent-encode '.' inside identifier tokens, so float values
        // appear as `4%2E1` rather than splitting into extra subject segments.
        assert!(matches_notification_filters(
            "extreme.4%2E1",
            &request,
            &constraints,
            None,
            ""
        ));
        assert!(!matches_notification_filters(
            "extreme.2%2E2",
            &request,
            &constraints,
            None,
            ""
        ));
    }

    #[test]
    fn test_matches_notification_filters_with_schema_key_different_from_topic_base() {
        ensure_global_schema_for_constraint_tests();
        let request = HashMap::new();
        let mut constraints = HashMap::new();
        constraints.insert(
            "step".to_string(),
            IdentifierConstraint::Int(NumericConstraint::Gte(4)),
        );

        assert!(matches_notification_filters(
            "diss.FOO.5",
            &request,
            &constraints,
            None,
            ""
        ));
        assert!(!matches_notification_filters(
            "diss.FOO.2",
            &request,
            &constraints,
            None,
            ""
        ));
    }
}
