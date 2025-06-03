//! Schema endpoint for retrieving notification configuration
//!
//! This module provides endpoints for querying the notification schema
//! configuration, allowing clients to discover available event types,
//! validation rules, and field requirements.
use crate::configuration::Settings;
use actix_web::{HttpResponse, web};
use serde_json::json;
use tracing::info;

/// Get the notification schema configuration
///
/// Returns the complete notification schema configuration including:
/// - All event types (dissemination, mars, etc.)
/// - Validation rules for each field
/// - Required vs optional fields
/// - Topic configuration
/// - Payload configuration
///
/// # Returns
/// * `200 OK` - Schema configuration with metadata
///   - `status`: "success"
///   - `schema`: Complete schema configuration
///   - `event_types`: List of available event type names
///   - `total_schemas`: Number of configured schemas
///   - `message`: Additional information (when no schema configured)
#[tracing::instrument]
pub async fn get_notification_schema() -> HttpResponse {
    // Get the global notification schema using zero-allocation access
    let schema = Settings::get_global_notification_schema();

    match schema {
        Some(schema_map) => {
            info!(
                schema_count = schema_map.len(),
                event_types = ?schema_map.keys().collect::<Vec<_>>(),
                "Returning notification schema configuration"
            );

            HttpResponse::Ok().json(json!({
                "status": "success",
                "schema": schema_map,
                "event_types": schema_map.keys().collect::<Vec<_>>(),
                "total_schemas": schema_map.len()
            }))
        }
        None => {
            info!("No notification schema configured, returning empty schema");

            HttpResponse::Ok().json(json!({
                "status": "success",
                "schema": {},
                "event_types": [],
                "total_schemas": 0,
                "message": "No notification schema configured"
            }))
        }
    }
}

/// Get schema for a specific event type
///
/// Returns the schema configuration for a single event type,
/// useful for clients that only need specific event information.
///
/// # Arguments
/// * `event_type` - Path parameter specifying the event type
///
/// # Returns
/// * `200 OK` - Event type schema found
/// * `404 Not Found` - Event type not configured
#[tracing::instrument]
pub async fn get_event_schema(path: web::Path<String>) -> HttpResponse {
    let event_type = path.into_inner();
    let schema = Settings::get_global_notification_schema();

    match schema {
        Some(schema_map) => {
            if let Some(event_schema) = schema_map.get(&event_type) {
                info!(
                    event_type = %event_type,
                    field_count = event_schema.request.len(),
                    "Returning schema for specific event type"
                );

                HttpResponse::Ok().json(json!({
                    "status": "success",
                    "event_type": event_type,
                    "schema": event_schema
                }))
            } else {
                HttpResponse::NotFound().json(json!({
                    "status": "error",
                    "message": format!("Event type '{}' not found", event_type),
                    "available_types": schema_map.keys().collect::<Vec<_>>()
                }))
            }
        }
        None => HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": "No notification schema configured"
        })),
    }
}
