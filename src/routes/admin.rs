use crate::notification_backend::NotificationBackend;
use actix_web::{HttpResponse, Result as ActixResult, web};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Deserialize)]
pub struct WipeStreamRequest {
    pub stream_name: String,
}

#[derive(Serialize)]
pub struct WipeResponse {
    pub success: bool,
    pub message: String,
}

/// Wipe an entire stream
pub async fn wipe_stream(
    backend: web::Data<Arc<dyn NotificationBackend>>,
    req: web::Json<WipeStreamRequest>,
) -> ActixResult<HttpResponse> {
    tracing::info!(stream_name = %req.stream_name, "Received request to wipe stream");

    match backend.wipe_stream(&req.stream_name).await {
        Ok(()) => {
            tracing::info!(stream_name = %req.stream_name, "Successfully wiped stream");
            Ok(HttpResponse::Ok().json(WipeResponse {
                success: true,
                message: format!("Successfully wiped stream: {}", req.stream_name),
            }))
        }
        Err(e) => {
            tracing::error!(stream_name = %req.stream_name, error = %e, "Failed to wipe stream");
            Ok(HttpResponse::InternalServerError().json(WipeResponse {
                success: false,
                message: format!("Failed to wipe stream: {}", e),
            }))
        }
    }
}

/// Wipe all data from all streams
pub async fn wipe_all(
    backend: web::Data<Arc<dyn NotificationBackend>>,
) -> ActixResult<HttpResponse> {
    tracing::warn!("Received request to wipe ALL data - this will remove everything!");

    match backend.wipe_all().await {
        Ok(()) => {
            tracing::warn!("Successfully wiped ALL data from backend");
            Ok(HttpResponse::Ok().json(WipeResponse {
                success: true,
                message: "Successfully wiped all data".to_string(),
            }))
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to wipe all data");
            Ok(HttpResponse::InternalServerError().json(WipeResponse {
                success: false,
                message: format!("Failed to wipe all data: {}", e),
            }))
        }
    }
}
