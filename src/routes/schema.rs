// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Schema endpoint for retrieving notification configuration
//!
//! This module provides endpoints for querying the notification schema
//! configuration, allowing clients to discover available event types,
//! identifier field descriptions, validation rules, and field requirements.
//! Internal configuration details like topic structure are excluded from API responses.

use crate::configuration::{ApiEventSchema, Settings};
use crate::telemetry::{SERVICE_NAME, SERVICE_VERSION};
use actix_web::{HttpResponse, web};
use serde_json::{Value, json};
use std::collections::HashMap;
use thiserror::Error;
use tracing::info;
use tracing_actix_web::RequestId;

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
#[tracing::instrument(skip(request_id), fields(request_id = %request_id))]
pub async fn get_event_schema(path: web::Path<String>, request_id: RequestId) -> HttpResponse {
    let event_type = path.into_inner();
    match lookup_event_schema(&event_type) {
        Ok(api_schema) => {
            info!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_name = "api.schema.get.succeeded",
                event_type = %event_type,
                field_count = api_schema.identifier.len(),
                "Returning filtered schema for specific event type"
            );
            HttpResponse::Ok().json(event_schema_body(&event_type, &api_schema))
        }
        Err(error) => {
            let mut body = json!({
                "status": "error",
                "message": error.to_string(),
                "request_id": request_id.to_string(),
            });
            if let EventSchemaError::NotFound { available, .. } = &error {
                body["available_types"] = json!(available);
            }
            HttpResponse::NotFound().json(body)
        }
    }
}

/// Why no public schema exists for an event type.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EventSchemaError {
    /// The schema map has no entry for the event type.
    #[error("Event type '{event_type}' not found")]
    NotFound {
        event_type: String,
        /// Configured event types, in map order.
        available: Vec<String>,
    },
    /// No notification schema is configured at all.
    #[error("No notification schema configured")]
    NoSchema,
}

/// The client-facing schema of one event type, without topic or endpoint
/// details.
pub fn lookup_event_schema(event_type: &str) -> Result<ApiEventSchema, EventSchemaError> {
    let schema_map = Settings::get_global_notification_schema()
        .as_ref()
        .ok_or(EventSchemaError::NoSchema)?;
    schema_map
        .get(event_type)
        .map(ApiEventSchema::from)
        .ok_or_else(|| EventSchemaError::NotFound {
            event_type: event_type.to_string(),
            available: schema_map.keys().cloned().collect(),
        })
}

/// Success body of `GET /api/v1/schema/{event_type}`.
pub fn event_schema_body(event_type: &str, schema: &ApiEventSchema) -> Value {
    json!({
        "status": "success",
        "event_type": event_type,
        "schema": schema
    })
}
