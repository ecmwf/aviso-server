// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use crate::notification::ProcessingResult;
use crate::notification::{
    SPATIAL_BBOX_METADATA_KEY, SPATIAL_GEOMETRY_METADATA_KEY, SPATIAL_POINT_CLOUD_METADATA_KEY,
    SpatialGeometry, decode_subject_for_display,
};
use crate::notification_backend::NotificationBackend;
use crate::telemetry::{SERVICE_NAME, SERVICE_VERSION};
use anyhow::Result;
use std::collections::HashMap;
use tracing::debug;

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
            SPATIAL_BBOX_METADATA_KEY.to_string(),
            spatial_metadata.bounding_box.clone(),
        );
        match &spatial_metadata.geometry {
            SpatialGeometry::Polygon(polygon) => {
                headers.insert(SPATIAL_GEOMETRY_METADATA_KEY.to_string(), polygon.clone());
            }
            SpatialGeometry::PointCloud(point_cloud) => {
                headers.insert(
                    SPATIAL_POINT_CLOUD_METADATA_KEY.to_string(),
                    point_cloud.clone(),
                );
            }
        }

        // Keep payload unchanged and attach spatial metadata via headers.
        notification_backend
            .put_message_with_headers(&result.topic, Some(headers), payload)
            .await?;

        debug!(
            service_name = SERVICE_NAME,
            service_version = SERVICE_VERSION,
            event_name = "notification.storage.spatial.succeeded",
            topic = %display_topic,
            event_type = %result.event_type,
            bounding_box = %spatial_metadata.bounding_box,
            "Notification with spatial metadata saved to backend successfully"
        );
    } else {
        notification_backend
            .put_messages(&result.topic, payload)
            .await?;

        debug!(
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

#[cfg(test)]
mod tests {
    use super::save_to_backend;
    use crate::notification::{
        ProcessingResult, SPATIAL_BBOX_METADATA_KEY, SPATIAL_GEOMETRY_METADATA_KEY,
        SPATIAL_POINT_CLOUD_METADATA_KEY, SpatialGeometry, spatial::SpatialMetadata,
    };
    use crate::notification_backend::{
        NotificationBackend,
        in_memory::{InMemoryBackend, InMemoryConfig},
        replay::{BatchParams, StartAt},
    };
    use std::collections::HashMap;

    #[test]
    fn spatial_header_keys_are_distinct() {
        assert_ne!(
            super::SPATIAL_GEOMETRY_METADATA_KEY,
            super::SPATIAL_POINT_CLOUD_METADATA_KEY
        );
    }

    #[tokio::test]
    async fn point_cloud_uses_dedicated_metadata_and_not_polygon_geometry() {
        let backend = InMemoryBackend::new(InMemoryConfig {
            max_history_per_topic: 10,
            max_topics: 10,
            enable_metrics: false,
        });
        let result = ProcessingResult {
            event_type: "cloud".to_string(),
            topic: "cloud.20260826".to_string(),
            canonicalized_params: HashMap::new(),
            identifier_constraints: HashMap::new(),
            spatial_metadata: Some(SpatialMetadata {
                bounding_box: "1,2,3,4".to_string(),
                geometry: SpatialGeometry::PointCloud("[[1.0,2.0],[3.0,4.0]]".to_string()),
            }),
            from_schema: true,
        };

        save_to_backend(&result, "null".to_string(), &backend)
            .await
            .expect("point cloud must store");
        let batch = backend
            .get_messages_batch(
                BatchParams::new("cloud.20260826".to_string(), 10)
                    .with_start_at(StartAt::Sequence(0)),
            )
            .await
            .expect("stored cloud batch");
        let metadata = batch.messages[0].metadata.as_ref().expect("metadata");
        assert_eq!(
            metadata.get(SPATIAL_POINT_CLOUD_METADATA_KEY),
            Some(&"[[1.0,2.0],[3.0,4.0]]".to_string())
        );
        assert_eq!(
            metadata.get(SPATIAL_BBOX_METADATA_KEY),
            Some(&"1,2,3,4".to_string())
        );
        assert!(!metadata.contains_key(SPATIAL_GEOMETRY_METADATA_KEY));
    }
}
