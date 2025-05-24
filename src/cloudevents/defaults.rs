//! Default value application for CloudEvents
//!
//! This module handles the application of sensible default values for
//! optional CloudEvent attributes that weren't provided by the client.
//! This ensures consistency and completeness across all processed events.

use anyhow::{Context, Result};
use chrono::Utc;
use cloudevents::{AttributesReader, Event, EventBuilder, EventBuilderV10};
use uuid::Uuid;

/// Apply default values to CloudEvent fields that are missing or empty
///
/// # Arguments
/// * `event` - The original CloudEvent that may have missing optional fields
///
/// # Returns
/// * `Ok(Event)` - CloudEvent with defaults applied
/// * `Err(anyhow::Error)` - Failed to apply defaults (rare, usually indicates corruption)
///
/// # Default Values Applied
/// - **ID**: UUID v4 if missing or empty (ensures uniqueness)
/// - **Source**: "/aviso-server" if missing or empty (identifies our service)
/// - **Time**: Current UTC timestamp if missing (when the event was processed)
/// - **Data Content Type**: "application/json" for JSON data if missing
///
/// # Design Rationale
/// Applying defaults ensures that:
/// - All events have unique identifiers for tracing and deduplication
/// - Event sources are consistently identified for routing and filtering
/// - Timestamps are available for ordering and time-based queries
/// - Data types are properly declared for consumer processing
pub fn apply_defaults(event: Event) -> Result<Event> {
    // Create a new builder from the existing event
    // This preserves all existing attributes while allowing us to modify specific ones
    let mut builder = EventBuilderV10::from(event.clone());

    // Apply default ID if missing or empty
    // We use UUID v4 to ensure global uniqueness across all event instances
    if event.id().is_empty() {
        let default_id = Uuid::new_v4().to_string();
        tracing::debug!(
            original_id = %event.id(),
            default_id = %default_id,
            "Applying default ID to CloudEvent"
        );
        builder = builder.id(default_id);
    }

    // Apply default source if missing or empty
    // The source identifies this Aviso server instance as the event processor
    if event.source().is_empty() {
        const DEFAULT_SOURCE: &str = "/aviso-server";
        tracing::debug!(
            original_source = %event.source(),
            default_source = DEFAULT_SOURCE,
            "Applying default source to CloudEvent"
        );
        builder = builder.source(DEFAULT_SOURCE);
    }

    // Apply default timestamp if missing
    // This records when the event was processed by our system
    if event.time().is_none() {
        let default_time = Utc::now();
        tracing::debug!(
            default_time = %default_time.to_rfc3339(),
            "Applying default timestamp to CloudEvent"
        );
        builder = builder.time(default_time);
    }

    // Apply default content type for JSON data if missing
    // This ensures consumers know how to interpret the event data
    if event.datacontenttype().is_none() && event.data().is_some() {
        if let Some(cloudevents::Data::Json(json_data)) = event.data() {
            const DEFAULT_CONTENT_TYPE: &str = "application/json";
            tracing::debug!(
                default_content_type = DEFAULT_CONTENT_TYPE,
                "Applying default content type for JSON data"
            );
            builder = builder.data(DEFAULT_CONTENT_TYPE, json_data.clone());
        }
    }

    // Build the final event with all defaults applied
    // The CloudEvents SDK validates the event structure during build
    builder.build().context(
        "Failed to build CloudEvent after applying defaults - this indicates event corruption",
    )
}
