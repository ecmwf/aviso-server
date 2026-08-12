// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use crate::auth::middleware::log_actor;
use crate::configuration::Settings;
use crate::notification_backend::{DeleteMessageResult, NotificationBackend, WipeStreamResult};
use crate::telemetry::{SERVICE_NAME, SERVICE_VERSION};
use actix_web::{HttpRequest, HttpResponse, Result as ActixResult, web};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing_actix_web::RequestId;
use utoipa::ToSchema;

#[derive(Deserialize, ToSchema)]
pub struct WipeStreamRequest {
    /// Event type or backend stream name, case-insensitive: "mars" and
    /// "MARS" both wipe the stream that serves the `mars` event type.
    pub stream_name: String,
}

#[derive(Serialize, ToSchema)]
pub struct WipeResponse {
    pub success: bool,
    pub message: String,
    /// Per-request UUID for log correlation. Same value as the
    /// `X-Request-ID` HTTP response header.
    pub request_id: String,
}

#[derive(Serialize, ToSchema)]
pub struct DeleteNotificationResponse {
    pub success: bool,
    pub message: String,
    pub notification_id: String,
    /// Per-request UUID for log correlation. Same value as the
    /// `X-Request-ID` HTTP response header.
    pub request_id: String,
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

/// Suffix for not-found messages naming the configured event types, so a
/// typo'd wipe request tells the operator what would have worked.
fn known_event_types_suffix() -> String {
    match Settings::get_global_notification_schema().as_ref() {
        Some(schema) if !schema.is_empty() => {
            let mut names: Vec<&str> = schema.keys().map(String::as_str).collect();
            names.sort_unstable();
            format!(". Known event types: {}", names.join(", "))
        }
        _ => String::new(),
    }
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
        (status = 404, description = "No stream by that name", body = WipeResponse),
        (status = 500, description = "Failed to wipe stream", body = WipeResponse),
        (status = 503, description = "Authentication service unavailable (direct mode)")
    ),
    security(
        ("bearer_jwt" = []),
        ("basic" = []),
    )
)]
#[tracing::instrument(
    skip(http_request, backend, req),
    fields(
        request_id = %request_id,
        username = tracing::field::Empty,
        auth_realm = tracing::field::Empty,
    )
)]
pub async fn wipe_stream(
    http_request: HttpRequest,
    backend: web::Data<Arc<dyn NotificationBackend>>,
    req: web::Json<WipeStreamRequest>,
    request_id: RequestId,
) -> ActixResult<HttpResponse> {
    let request_id_str = request_id.to_string();
    // Attribute the destructive operation (and the backend-layer events it
    // triggers, which inherit span fields) to the authenticated identity.
    let (username, auth_realm) = log_actor(&http_request);
    tracing::Span::current().record("username", username.as_str());
    tracing::Span::current().record("auth_realm", auth_realm.as_str());
    let resolved_stream_key = resolve_stream_key_alias(&req.stream_name);
    tracing::info!(
        service_name = SERVICE_NAME,
        service_version = SERVICE_VERSION,
        event_name = "admin.stream.wipe.requested",
        stream_name = %req.stream_name,
        stream_key = %resolved_stream_key,
        request_id = %request_id_str,
        "Received request to wipe stream"
    );

    match backend.wipe_stream(&resolved_stream_key).await {
        Ok(WipeStreamResult::Wiped) => {
            tracing::info!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_name = "admin.stream.wipe.succeeded",
                stream_name = %req.stream_name,
                stream_key = %resolved_stream_key,
                request_id = %request_id_str,
                "Successfully wiped stream"
            );
            Ok(HttpResponse::Ok().json(WipeResponse {
                success: true,
                message: format!("Successfully wiped stream: {}", req.stream_name),
                request_id: request_id_str,
            }))
        }
        Ok(WipeStreamResult::NotFound) => {
            tracing::warn!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_name = "admin.stream.wipe.not_found",
                stream_name = %req.stream_name,
                stream_key = %resolved_stream_key,
                request_id = %request_id_str,
                "Stream not found"
            );
            Ok(HttpResponse::NotFound().json(WipeResponse {
                success: false,
                message: format!(
                    "Stream not found: {}{}",
                    req.stream_name,
                    known_event_types_suffix()
                ),
                request_id: request_id_str,
            }))
        }
        Err(e) => {
            tracing::error!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_name = "admin.stream.wipe.failed",
                stream_name = %req.stream_name,
                stream_key = %resolved_stream_key,
                error = %e,
                request_id = %request_id_str,
                "Failed to wipe stream"
            );
            Ok(HttpResponse::InternalServerError().json(WipeResponse {
                success: false,
                message: format!("Failed to wipe stream: {}", e),
                request_id: request_id_str,
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
#[tracing::instrument(
    skip(http_request, backend),
    fields(
        request_id = %request_id,
        username = tracing::field::Empty,
        auth_realm = tracing::field::Empty,
    )
)]
pub async fn wipe_all(
    http_request: HttpRequest,
    backend: web::Data<Arc<dyn NotificationBackend>>,
    request_id: RequestId,
) -> ActixResult<HttpResponse> {
    let request_id_str = request_id.to_string();
    // Attribute the destructive operation (and the backend-layer events it
    // triggers, which inherit span fields) to the authenticated identity.
    let (username, auth_realm) = log_actor(&http_request);
    tracing::Span::current().record("username", username.as_str());
    tracing::Span::current().record("auth_realm", auth_realm.as_str());
    tracing::warn!(
        service_name = SERVICE_NAME,
        service_version = SERVICE_VERSION,
        event_name = "admin.all.wipe.requested",
        request_id = %request_id_str,
        "Received request to wipe ALL data - this will remove everything!"
    );

    match backend.wipe_all().await {
        Ok(()) => {
            tracing::warn!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_name = "admin.all.wipe.succeeded",
                request_id = %request_id_str,
                "Successfully wiped ALL data from backend"
            );
            Ok(HttpResponse::Ok().json(WipeResponse {
                success: true,
                message: "Successfully wiped all data".to_string(),
                request_id: request_id_str,
            }))
        }
        Err(e) => {
            tracing::error!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_name = "admin.all.wipe.failed",
                error = %e,
                request_id = %request_id_str,
                "Failed to wipe all data"
            );
            Ok(HttpResponse::InternalServerError().json(WipeResponse {
                success: false,
                message: format!("Failed to wipe all data: {}", e),
                request_id: request_id_str,
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
#[tracing::instrument(
    skip(http_request, backend, path),
    fields(
        request_id = %request_id,
        username = tracing::field::Empty,
        auth_realm = tracing::field::Empty,
    )
)]
pub async fn delete_notification(
    http_request: HttpRequest,
    backend: web::Data<Arc<dyn NotificationBackend>>,
    path: web::Path<String>,
    request_id: RequestId,
) -> ActixResult<HttpResponse> {
    let request_id_str = request_id.to_string();
    // Attribute the destructive operation (and the backend-layer events it
    // triggers, which inherit span fields) to the authenticated identity.
    let (username, auth_realm) = log_actor(&http_request);
    tracing::Span::current().record("username", username.as_str());
    tracing::Span::current().record("auth_realm", auth_realm.as_str());
    let raw_id = path.into_inner();
    let parsed = match parse_notification_id(&raw_id) {
        Ok(parsed) => parsed,
        Err(message) => {
            tracing::warn!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_name = "admin.notification.delete.invalid_id",
                notification_id = %raw_id,
                request_id = %request_id_str,
                "Invalid notification ID format"
            );
            return Ok(HttpResponse::BadRequest().json(DeleteNotificationResponse {
                success: false,
                message: message.to_string(),
                notification_id: raw_id,
                request_id: request_id_str,
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
                request_id = %request_id_str,
                "Deleted notification"
            );
            Ok(HttpResponse::Ok().json(DeleteNotificationResponse {
                success: true,
                message: "Notification deleted".to_string(),
                notification_id: raw_id,
                request_id: request_id_str,
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
                request_id = %request_id_str,
                "Notification not found"
            );
            Ok(HttpResponse::NotFound().json(DeleteNotificationResponse {
                success: false,
                message: "Notification not found".to_string(),
                notification_id: raw_id,
                request_id: request_id_str,
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
                request_id = %request_id_str,
                "Failed to delete notification"
            );
            Ok(
                HttpResponse::InternalServerError().json(DeleteNotificationResponse {
                    success: false,
                    message: format!("Failed to delete notification: {error}"),
                    notification_id: raw_id,
                    request_id: request_id_str,
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
