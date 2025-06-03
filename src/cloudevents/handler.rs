//! CloudEvent request handler
//!
//! This module provides the core CloudEvent processing logic that can be
//! reused across multiple endpoints. It handles the complete CloudEvent
//! lifecycle from JSON payload to processed event.

use anyhow::{Context, Result};
use serde_json::Value;
use tracing::info;

use super::CloudEventProcessor;
use cloudevents::AttributesReader;

/// Response from processing a CloudEvent
#[derive(Debug, Clone, serde::Serialize)]
pub struct CloudEventResponse {
    /// Processing status indicator
    ///
    /// Always "success" for successful processing.
    /// This is for clients to quickly determine if
    /// CloudEvent was processed successfully without parsing error details.
    pub status: String,
    /// Unique identifier of the processed CloudEvent
    ///
    /// This is the CloudEvent's 'id' attribute, which uniquely identifies
    /// this specific event instance. Clients can use this ID for:
    /// - Tracking and correlation in logs
    /// - Deduplication of events
    /// - Referencing this event in subsequent operations
    /// - Debugging and troubleshooting
    pub event_id: String,
    /// Type of the CloudEvent that was processed
    ///
    /// This corresponds to the CloudEvent's 'type' attribute (e.g., "aviso").
    /// This describes the nature of the event:
    /// - What kind of event was processed
    /// - How to interpret the event data
    /// - Which business logic was triggered
    /// - Event categorization for monitoring and analytics
    pub event_type: String,
    /// Source context where the CloudEvent originated
    ///
    /// This is the CloudEvent's 'source' attribute (e.g., "/host/user").
    /// It identifies the context in which the event happened and helps with:
    /// - Event routing and filtering
    /// - Understanding the event's origin
    /// - Implementing source-based access controls
    /// - Organizing events by their producing systems
    pub event_source: String,
    /// Human-readable description of the processing result
    ///
    /// Provides a clear, descriptive message about what happened during
    /// processing. This is useful for:
    /// - User interfaces displaying processing status
    /// - Logging and monitoring dashboards
    /// - Debugging when things go wrong
    /// - Providing context to operators and developers
    pub message: String,
    /// ISO 8601 timestamp when the event processing completed
    ///
    /// Records exactly when the aviso server finished processing this event.
    /// This timestamp is useful for:
    /// - Performance monitoring and SLA tracking
    /// - Ordering events by processing time
    /// - Debugging timing-related issues
    /// - Audit trails and compliance reporting
    ///
    /// Format: "2025-05-25T12:00:00.000Z" (RFC 3339)
    pub processed_at: String,
}

/// Handle CloudEvent processing from JSON payload
///
/// This function encapsulates the complete CloudEvent processing workflow:
/// - Parse and validate the CloudEvent from JSON
/// - Apply default values for missing optional fields
/// - Return structured response data
///
/// This handler can be reused across multiple endpoints that need to process
/// CloudEvents, providing consistent behavior and response format.
///
/// # Arguments
/// * `json_payload` - The JSON payload containing the CloudEvent
///
/// # Returns
/// * `Ok(CloudEventResponse)` - Successfully processed CloudEvent
/// * `Err(anyhow::Error)` - Processing failed with detailed error information
pub async fn handle_cloudevent(json_payload: Value) -> Result<CloudEventResponse> {
    // Process the CloudEvent using our dedicated processor
    // This handles parsing, default application, and validation in one step
    let event = CloudEventProcessor::process_json_payload(json_payload)
        .context("Failed to process CloudEvent from JSON payload")?;

    info!(
        event_id = %event.id(),
        event_type = %event.ty(),
        event_source = %event.source(),
        "Successfully processed CloudEvent"
    );

    // Log successful processing with all relevant metadata
    info!(
        event_id = %event.id(),
        event_type = %event.ty(),
        source = %event.source(),
        timestamp = ?event.time(),
        "CloudEvent processed successfully"
    );

    // Return structured response data
    Ok(CloudEventResponse {
        status: "success".to_string(),
        event_id: event.id().to_string(),
        event_type: event.ty().to_string(),
        event_source: event.source().to_string(),
        message: "CloudEvent processed successfully".to_string(),
        processed_at: chrono::Utc::now().to_rfc3339(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_handle_valid_cloudevent() {
        let cloudevent_payload = json!({
            "type": "aviso",
            "data": {
                "event": "dissemination",
                "request": {
                    "class": "od",
                    "target": "E1",
                    "date": "20000101",
                    "destination": "SCL",
                    "domain": "g",
                    "expver": "9999",
                    "step": "1",
                    "stream": "enfo",
                    "time": "0"
                },
                "location": "xyz"
            },
            "datacontenttype": "application/json",
            "id": "test-event-123",
            "source": "/host/user",
            "specversion": "1.0",
            "time": "2000-01-01T00:00:00.000Z"
        });

        let result = handle_cloudevent(cloudevent_payload).await;
        assert!(result.is_ok());

        let response = result.unwrap();
        assert_eq!(response.status, "success");
        assert_eq!(response.event_id, "test-event-123");
        assert_eq!(response.event_type, "aviso");
        assert_eq!(response.event_source, "/host/user");
    }

    #[tokio::test]
    async fn test_handle_invalid_cloudevent() {
        let invalid_payload = json!({
            "invalid": "format"
        });

        let result = handle_cloudevent(invalid_payload).await;
        assert!(result.is_err());
    }
}
