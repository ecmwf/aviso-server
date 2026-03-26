// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use crate::notification::ProcessingResult;
use crate::notification::decode_subject_for_display;
use crate::notification_backend::NotificationBackend;
use crate::telemetry::{SERVICE_NAME, SERVICE_VERSION};
use anyhow::Result;
use aviso_validators::PolygonHandler;
use std::collections::HashMap;
use tracing::{debug, info};

/// Save notification result to the configured backend
///
/// Takes a processed notification result and persists it to the backend storage.
/// Spatial metadata, when present, is attached via backend headers.
#[tracing::instrument(
    skip(payload, notification_backend),
    fields(
        topic = %result.topic,
        event_type = %result.event_type,
        spatial_enabled = result.spatial_metadata.is_some(),
        payload_len = payload.len(),
    )
)]
pub async fn save_to_backend(
    result: &ProcessingResult,
    payload: String,
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

        // Keep payload unchanged and attach spatial metadata via headers.
        notification_backend
            .put_message_with_headers(&result.topic, Some(headers), payload)
            .await?;

        info!(
            service_name = SERVICE_NAME,
            service_version = SERVICE_VERSION,
            event_name = "notification.storage.spatial.succeeded",
            topic = %display_topic,
            event_type = %result.event_type,
            bounding_box = %spatial_metadata.bounding_box,
            "Notification with spatial metadata saved to backend successfully"
        );
    } else {
        // Save the notification result to backend using put_messages
        notification_backend
            .put_messages(&result.topic, payload)
            .await?;

        info!(
            service_name = SERVICE_NAME,
            service_version = SERVICE_VERSION,
            event_name = "notification.storage.succeeded",
            topic = %display_topic,
            event_type = %result.event_type,
            "Notification saved to backend successfully"
        );
    }

    Ok(())
}

fn find_polygon_geometry(
    canonicalized_params: &HashMap<String, String>,
) -> Option<(String, Vec<(f64, f64)>)> {
    let polygon = canonicalized_params.get("polygon")?;
    let coordinates = PolygonHandler::parse_polygon_coordinates(polygon).ok()?;
    Some((polygon.clone(), coordinates))
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
    fn does_not_extract_polygon_from_non_polygon_key() {
        let mut params = HashMap::new();
        params.insert(
            "shape".to_string(),
            "(52.5,13.4,52.6,13.5,52.5,13.6,52.4,13.5,52.5,13.4)".to_string(),
        );

        assert!(find_polygon_geometry(&params).is_none());
    }

    #[test]
    fn returns_none_when_no_valid_polygon_value_exists() {
        let mut params = HashMap::new();
        params.insert("shape".to_string(), "(not-a-polygon)".to_string());
        params.insert("time".to_string(), "1200".to_string());
        assert!(find_polygon_geometry(&params).is_none());
    }
}
