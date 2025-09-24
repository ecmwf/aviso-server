use crate::notification::ProcessingResult;
use crate::notification_backend::NotificationBackend;
use anyhow::Result;
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
    debug!(
        topic = %result.topic,
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

        // Enhance payload with full polygon geometry from request params
        let enhanced_payload = enhance_payload_with_polygon(base_payload, result)?;

        // Save with spatial headers and enhanced payload
        notification_backend
            .put_message_with_headers(&result.topic, Some(headers), enhanced_payload)
            .await?;

        info!(
            topic = %result.topic,
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
            topic = %result.topic,
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
    result: &ProcessingResult,
) -> Result<String> {
    // Parse original payload as JSON
    let mut payload_json: serde_json::Value = serde_json::from_str(original_payload)?;

    // Find polygon field from canonicalized params
    let polygon_coordinates = result
        .canonicalized_params
        .iter()
        .find(|(_, value)| value.starts_with('(') && value.ends_with(')'))
        .map(|(_, value)| {
            crate::notification::validators::PolygonHandler::parse_polygon_coordinates(value)
        })
        .transpose()?;

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
