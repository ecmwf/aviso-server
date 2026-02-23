use crate::notification::ProcessingResult;
use crate::notification::decode_subject_for_display;
use crate::notification_backend::NotificationBackend;
use anyhow::Result;
use aviso_validators::PolygonHandler;
use std::collections::HashMap;
use tracing::{debug, info};

/// Save notification result to the configured backend
///
/// Takes a processed notification result and persists it to the backend storage.
/// The result contains the validated topic, canonicalized parameters, and payload.
/// Now supports spatial metadata for polygon-enabled notifications.
#[tracing::instrument(
    skip(notification_backend),
    fields(
        topic = %result.topic,
        event_type = %result.event_type,
        spatial_enabled = result.spatial_metadata.is_some(),
    )
)]
pub async fn save_to_backend(
    result: &ProcessingResult,
    payload: Option<&str>,
    notification_backend: &dyn NotificationBackend,
) -> Result<()> {
    let display_topic = decode_subject_for_display(&result.topic);
    debug!(
        topic = %display_topic,
        event_type = %result.event_type,
        param_count = result.canonicalized_params.len(),
        has_spatial_metadata = result.spatial_metadata.is_some(),
        "Saving notification to backend"
    );

    let base_payload = payload.unwrap_or("");

    // Check if spatial metadata exists and use appropriate backend method
    if let Some(spatial_metadata) = &result.spatial_metadata {
        // Create headers for spatial data (backend doesn't know what spatial_bbox means)
        let mut headers = HashMap::new();
        headers.insert(
            "spatial_bbox".to_string(),
            spatial_metadata.bounding_box.clone(),
        );
        let polygon_geometry = find_polygon_geometry(&result.canonicalized_params);
        if let Some((polygon_geometry, _)) = &polygon_geometry {
            headers.insert("spatial_geometry".to_string(), polygon_geometry.clone());
        }

        // Enhance payload with full polygon geometry from request params
        let enhanced_payload = enhance_payload_with_polygon(
            base_payload,
            polygon_geometry.as_ref().map(|(_, c)| c.as_slice()),
        )?;

        // Save with spatial headers and enhanced payload
        notification_backend
            .put_message_with_headers(&result.topic, Some(headers), enhanced_payload)
            .await?;

        info!(
            topic = %display_topic,
            event_type = %result.event_type,
            bounding_box = %spatial_metadata.bounding_box,
            "Notification with spatial metadata saved to backend successfully"
        );
    } else {
        // Save the notification result to backend using put_messages
        notification_backend
            .put_messages(&result.topic, base_payload.to_string())
            .await?;

        info!(
            topic = %display_topic,
            event_type = %result.event_type,
            "Notification saved to backend successfully"
        );
    }

    Ok(())
}

/// Enhance payload JSON with full polygon geometry
///
/// Takes the original payload and adds spatial geometry information for precise
/// intersection calculations during watch filtering. Extracts polygon coordinates
/// from the canonicalized request parameters.
///
/// # Arguments
/// * `original_payload` - The original payload as JSON string
/// * `result` - Processing result containing canonicalized parameters
///
/// # Returns
/// * `Ok(String)` - Enhanced payload JSON with spatial_geometry field
/// * `Err(anyhow::Error)` - JSON parsing or polygon extraction failed
fn enhance_payload_with_polygon(
    original_payload: &str,
    polygon_coordinates: Option<&[(f64, f64)]>,
) -> Result<String> {
    // Parse original payload as JSON
    let mut payload_json: serde_json::Value = serde_json::from_str(original_payload)?;

    if let Some(coordinates) = polygon_coordinates {
        // HERE: Make sure this is lat,lon order for GeoJSON!
        if let Some(payload_obj) = payload_json.as_object_mut() {
            let spatial_geometry = serde_json::json!({
                "type": "Polygon",
                // GeoJSON wants [ [ [lon,lat], ... ] ], but we want [ [ [lat,lon], ... ] ]
                // Just use coordinates as-is, since whole stack is lat,lon
                "coordinates": [coordinates]
            });

            payload_obj.insert("spatial_geometry".to_string(), spatial_geometry);

            tracing::debug!(
                coordinate_count = coordinates.len(),
                "Enhanced payload with polygon geometry"
            );
        }
    }

    Ok(serde_json::to_string(&payload_json)?)
}

fn find_polygon_geometry(
    canonicalized_params: &HashMap<String, String>,
) -> Option<(String, Vec<(f64, f64)>)> {
    canonicalized_params.values().find_map(|value| {
        PolygonHandler::parse_polygon_coordinates(value)
            .ok()
            .map(|coordinates| (value.clone(), coordinates))
    })
}

#[cfg(test)]
mod tests {
    use super::find_polygon_geometry;
    use std::collections::HashMap;

    #[test]
    fn finds_valid_polygon_and_ignores_non_polygon_parenthesized_values() {
        let mut params = HashMap::new();
        params.insert("foo".to_string(), "(not-a-polygon)".to_string());
        params.insert(
            "polygon".to_string(),
            "(52.5,13.4,52.6,13.5,52.5,13.6,52.4,13.5,52.5,13.4)".to_string(),
        );

        let found = find_polygon_geometry(&params);
        assert!(found.is_some());
        let (raw, coordinates) = found.expect("must find polygon geometry");
        assert!(raw.starts_with('(') && raw.ends_with(')'));
        assert_eq!(coordinates.len(), 5);
    }

    #[test]
    fn returns_none_when_no_valid_polygon_value_exists() {
        let mut params = HashMap::new();
        params.insert("shape".to_string(), "(not-a-polygon)".to_string());
        params.insert("time".to_string(), "1200".to_string());
        assert!(find_polygon_geometry(&params).is_none());
    }
}
