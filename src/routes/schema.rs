//! Schema endpoint for retrieving notification configuration
//!
//! This module provides endpoints for querying the notification schema
//! configuration, allowing clients to discover available event types,
//! identifier field descriptions, validation rules, and field requirements.
//! Internal configuration details like topic structure are excluded from API responses.

use crate::configuration::{ApiEventSchema, Settings};
use crate::telemetry::{SERVICE_NAME, SERVICE_VERSION};
use actix_web::{HttpResponse, web};
use serde_json::json;
use std::collections::HashMap;
use tracing::info;

/// Get the notification schema configuration
///
/// Returns a filtered notification schema configuration including:
/// - All event types (dissemination, mars, etc.)
/// - Optional description for each identifier field
/// - Validation rules for each identifier field
/// - Required vs optional fields
/// - Payload configuration
///
/// Note: Topic and endpoint configuration are excluded from the response
/// as they are internal implementation details.
///
/// # Returns
/// * `200 OK` - Filtered schema configuration with metadata
///   - `status`: "success"
///   - `schema`: Schema configuration (without topic/endpoint fields)
///   - `event_types`: List of available event type names
///   - `total_schemas`: Number of configured schemas
///   - `message`: Additional information (when no schema configured)
#[utoipa::path(get, path = "/api/v1/schema", tag = "schema")]
#[tracing::instrument]
pub async fn get_notification_schema() -> HttpResponse {
    // Get the global notification schema using zero-allocation access
    let schema = Settings::get_global_notification_schema();

    match schema {
        Some(schema_map) => {
            // Convert to API-friendly format (excluding topic and endpoint)
            let api_schema: HashMap<String, ApiEventSchema> = schema_map
                .iter()
                .map(|(key, value)| (key.clone(), ApiEventSchema::from(value)))
                .collect();

            info!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_name = "api.schema.list.succeeded",
                schema_count = api_schema.len(),
                event_types = ?api_schema.keys().collect::<Vec<_>>(),
                "Returning filtered notification schema configuration"
            );

            HttpResponse::Ok().json(json!({
                "status": "success",
                "schema": api_schema,
                "event_types": api_schema.keys().collect::<Vec<_>>(),
                "total_schemas": api_schema.len()
            }))
        }
        None => {
            info!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_name = "api.schema.list.empty",
                "No notification schema configured, returning empty schema"
            );

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
/// Returns the filtered schema configuration for a single event type,
/// useful for clients that only need specific event information.
/// Topic and endpoint configuration are excluded from the response.
///
/// # Arguments
/// * `event_type` - Path parameter specifying the event type
///
/// # Returns
/// * `200 OK` - Event type schema found (filtered)
/// * `404 Not Found` - Event type not configured
#[utoipa::path(
    get,
    path = "/api/v1/schema/{event_type}",
    tag = "schema",
    params(
        ("event_type" = String, Path, description = "Event type identifier")
    ),
)]
#[tracing::instrument]
pub async fn get_event_schema(path: web::Path<String>) -> HttpResponse {
    let event_type = path.into_inner();
    let schema = Settings::get_global_notification_schema();

    match schema {
        Some(schema_map) => {
            if let Some(event_schema) = schema_map.get(&event_type) {
                // Convert to API-friendly format
                let api_schema = ApiEventSchema::from(event_schema);

                info!(
                    service_name = SERVICE_NAME,
                    service_version = SERVICE_VERSION,
                    event_name = "api.schema.get.succeeded",
                    event_type = %event_type,
                    field_count = api_schema.identifier.len(),
                    "Returning filtered schema for specific event type"
                );

                HttpResponse::Ok().json(json!({
                    "status": "success",
                    "event_type": event_type,
                    "schema": api_schema
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
