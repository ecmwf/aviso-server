use crate::notification_backend::NotificationBackend;
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

/// Wipe an entire stream
#[utoipa::path(
    delete,
    path = "/api/v1/admin/wipe/stream",
    tag = "admin",
    request_body = WipeStreamRequest,
    responses(
        (status = 200, description = "Stream wiped successfully", body = WipeResponse),
        (status = 500, description = "Failed to wipe stream", body = WipeResponse)
    )
)]
pub async fn wipe_stream(
    backend: web::Data<Arc<dyn NotificationBackend>>,
    req: web::Json<WipeStreamRequest>,
) -> ActixResult<HttpResponse> {
    tracing::info!(
        service_name = SERVICE_NAME,
        service_version = SERVICE_VERSION,
        event_domain = "admin",
        event_name = "admin.stream.wipe.requested",
        stream_name = %req.stream_name,
        "Received request to wipe stream"
    );

    match backend.wipe_stream(&req.stream_name).await {
        Ok(()) => {
            tracing::info!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_domain = "admin",
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
                event_domain = "admin",
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
        (status = 500, description = "Failed to wipe all data", body = WipeResponse)
    )
)]

pub async fn wipe_all(
    backend: web::Data<Arc<dyn NotificationBackend>>,
) -> ActixResult<HttpResponse> {
    tracing::warn!(
        service_name = SERVICE_NAME,
        service_version = SERVICE_VERSION,
        event_domain = "admin",
        event_name = "admin.all.wipe.requested",
        "Received request to wipe ALL data - this will remove everything!"
    );

    match backend.wipe_all().await {
        Ok(()) => {
            tracing::warn!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_domain = "admin",
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
                event_domain = "admin",
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
