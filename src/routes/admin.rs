// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use crate::configuration::Settings;
use crate::notification_backend::{DeleteMessageResult, NotificationBackend};
use crate::telemetry::{SERVICE_NAME, SERVICE_VERSION};
use actix_web::{HttpResponse, Result as ActixResult, web};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;

#[derive(Deserialize, ToSchema)]
pub struct WipeStreamRequest {
    pub stream_name: String,
}

#[derive(Serialize, ToSchema)]
pub struct WipeResponse {
    pub success: bool,
    pub message: String,
}

#[derive(Serialize, ToSchema)]
pub struct DeleteNotificationResponse {
    pub success: bool,
    pub message: String,
    pub notification_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedNotificationId {
    stream_key: String,
    sequence: u64,
}

fn parse_notification_id(value: &str) -> Result<ParsedNotificationId, &'static str> {
    let trimmed = value.trim();
    let (raw_stream_key, raw_sequence) = trimmed
        .split_once('@')
        .ok_or("notification_id must be in '<stream>@<sequence>' format")?;
    if raw_stream_key.is_empty() || raw_sequence.is_empty() {
        return Err("notification_id must be in '<stream>@<sequence>' format");
    }
    let sequence = raw_sequence
        .parse::<u64>()
        .map_err(|_| "notification_id sequence must be a positive integer")?;
    if sequence == 0 {
        return Err("notification_id sequence must be greater than zero");
    }

    Ok(ParsedNotificationId {
        stream_key: raw_stream_key.to_string(),
        sequence,
    })
}

fn resolve_stream_key_alias(stream_or_event_type: &str) -> String {
    let Some(schema) = Settings::get_global_notification_schema().as_ref() else {
        return stream_or_event_type.to_string();
    };
    let event_schema = schema.get(stream_or_event_type).or_else(|| {
        schema.iter().find_map(|(event_type, schema)| {
            if event_type.eq_ignore_ascii_case(stream_or_event_type) {
                Some(schema)
            } else {
                None
            }
        })
    });
    let Some(event_schema) = event_schema else {
        return stream_or_event_type.to_string();
    };
    event_schema
        .topic
        .as_ref()
        .map(|topic| topic.base.clone())
        .unwrap_or_else(|| stream_or_event_type.to_string())
}

/// Wipe an entire stream
#[utoipa::path(
    delete,
    path = "/api/v1/admin/wipe/stream",
    tag = "admin",
    request_body = WipeStreamRequest,
    responses(
        (status = 200, description = "Stream wiped successfully", body = WipeResponse),
        (status = 401, description = "Missing or invalid credentials"),
        (status = 403, description = "Valid credentials but missing admin role"),
        (status = 500, description = "Failed to wipe stream", body = WipeResponse),
        (status = 503, description = "Authentication service unavailable (direct mode)")
    ),
    security(
        ("bearer_jwt" = []),
        ("basic" = []),
    )
)]
pub async fn wipe_stream(
    backend: web::Data<Arc<dyn NotificationBackend>>,
    req: web::Json<WipeStreamRequest>,
) -> ActixResult<HttpResponse> {
    tracing::info!(
        service_name = SERVICE_NAME,
        service_version = SERVICE_VERSION,
        event_name = "admin.stream.wipe.requested",
        stream_name = %req.stream_name,
        "Received request to wipe stream"
    );

    match backend.wipe_stream(&req.stream_name).await {
        Ok(()) => {
            tracing::info!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_name = "admin.stream.wipe.succeeded",
                stream_name = %req.stream_name,
                "Successfully wiped stream"
            );
            Ok(HttpResponse::Ok().json(WipeResponse {
                success: true,
                message: format!("Successfully wiped stream: {}", req.stream_name),
            }))
        }
        Err(e) => {
            tracing::error!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_name = "admin.stream.wipe.failed",
                stream_name = %req.stream_name,
                error = %e,
                "Failed to wipe stream"
            );
            Ok(HttpResponse::InternalServerError().json(WipeResponse {
                success: false,
                message: format!("Failed to wipe stream: {}", e),
            }))
        }
    }
}

/// Wipe all data from all streams
#[utoipa::path(
    delete,
    path = "/api/v1/admin/wipe/all",
    tag = "admin",
    responses(
        (status = 200, description = "All data wiped successfully", body = WipeResponse),
        (status = 401, description = "Missing or invalid credentials"),
        (status = 403, description = "Valid credentials but missing admin role"),
        (status = 500, description = "Failed to wipe all data", body = WipeResponse),
        (status = 503, description = "Authentication service unavailable (direct mode)")
    ),
    security(
        ("bearer_jwt" = []),
        ("basic" = []),
    )
)]

pub async fn wipe_all(
    backend: web::Data<Arc<dyn NotificationBackend>>,
) -> ActixResult<HttpResponse> {
    tracing::warn!(
        service_name = SERVICE_NAME,
        service_version = SERVICE_VERSION,
        event_name = "admin.all.wipe.requested",
        "Received request to wipe ALL data - this will remove everything!"
    );

    match backend.wipe_all().await {
        Ok(()) => {
            tracing::warn!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_name = "admin.all.wipe.succeeded",
                "Successfully wiped ALL data from backend"
            );
            Ok(HttpResponse::Ok().json(WipeResponse {
                success: true,
                message: "Successfully wiped all data".to_string(),
            }))
        }
        Err(e) => {
            tracing::error!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_name = "admin.all.wipe.failed",
                error = %e,
                "Failed to wipe all data"
            );
            Ok(HttpResponse::InternalServerError().json(WipeResponse {
                success: false,
                message: format!("Failed to wipe all data: {}", e),
            }))
        }
    }
}

/// Delete one notification by `<stream_or_event_type>@<sequence>`
#[utoipa::path(
    delete,
    path = "/api/v1/admin/notification/{notification_id}",
    tag = "admin",
    params(
        ("notification_id" = String, Path, description = "Notification identifier in the form '<stream_or_event_type>@<sequence>'")
    ),
    responses(
        (status = 200, description = "Notification deleted", body = DeleteNotificationResponse),
        (status = 400, description = "Invalid notification ID format", body = DeleteNotificationResponse),
        (status = 401, description = "Missing or invalid credentials"),
        (status = 403, description = "Valid credentials but missing admin role"),
        (status = 404, description = "Notification not found", body = DeleteNotificationResponse),
        (status = 500, description = "Delete operation failed", body = DeleteNotificationResponse),
        (status = 503, description = "Authentication service unavailable (direct mode)")
    ),
    security(
        ("bearer_jwt" = []),
        ("basic" = []),
    )
)]
pub async fn delete_notification(
    backend: web::Data<Arc<dyn NotificationBackend>>,
    path: web::Path<String>,
) -> ActixResult<HttpResponse> {
    let raw_id = path.into_inner();
    let parsed = match parse_notification_id(&raw_id) {
        Ok(parsed) => parsed,
        Err(message) => {
            tracing::warn!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_name = "admin.notification.delete.invalid_id",
                notification_id = %raw_id,
                "Invalid notification ID format"
            );
            return Ok(HttpResponse::BadRequest().json(DeleteNotificationResponse {
                success: false,
                message: message.to_string(),
                notification_id: raw_id,
            }));
        }
    };
    let resolved_stream_key = resolve_stream_key_alias(&parsed.stream_key);

    match backend
        .delete_message(&resolved_stream_key, parsed.sequence)
        .await
    {
        Ok(DeleteMessageResult::Deleted) => {
            tracing::info!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_name = "admin.notification.delete.succeeded",
                notification_id = %raw_id,
                stream_key = %resolved_stream_key,
                sequence = parsed.sequence,
                "Deleted notification"
            );
            Ok(HttpResponse::Ok().json(DeleteNotificationResponse {
                success: true,
                message: "Notification deleted".to_string(),
                notification_id: raw_id,
            }))
        }
        Ok(DeleteMessageResult::NotFound) => {
            tracing::warn!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_name = "admin.notification.delete.not_found",
                notification_id = %raw_id,
                stream_key = %resolved_stream_key,
                sequence = parsed.sequence,
                "Notification not found"
            );
            Ok(HttpResponse::NotFound().json(DeleteNotificationResponse {
                success: false,
                message: "Notification not found".to_string(),
                notification_id: raw_id,
            }))
        }
        Err(error) => {
            tracing::error!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_name = "admin.notification.delete.failed",
                notification_id = %raw_id,
                stream_key = %resolved_stream_key,
                sequence = parsed.sequence,
                error = %error,
                "Failed to delete notification"
            );
            Ok(
                HttpResponse::InternalServerError().json(DeleteNotificationResponse {
                    success: false,
                    message: format!("Failed to delete notification: {error}"),
                    notification_id: raw_id,
                }),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_notification_id;

    #[test]
    fn parses_valid_notification_id() {
        let parsed = parse_notification_id("test_polygon@42").expect("valid id should parse");
        assert_eq!(parsed.stream_key, "test_polygon");
        assert_eq!(parsed.sequence, 42);
    }

    #[test]
    fn rejects_missing_separator() {
        let error = parse_notification_id("test_polygon42").expect_err("must fail");
        assert_eq!(
            error,
            "notification_id must be in '<stream>@<sequence>' format"
        );
    }

    #[test]
    fn rejects_zero_sequence() {
        let error = parse_notification_id("test_polygon@0").expect_err("must fail");
        assert_eq!(error, "notification_id sequence must be greater than zero");
    }
}
