use crate::cloudevents::handler::handle_cloudevent;
use crate::error::validation_error_response;
use crate::notification_backend::NotificationBackend;
use actix_web::{HttpResponse, web};
use serde_json::Value;
use std::sync::Arc;

#[tracing::instrument(skip(backend, payload))]
pub async fn notify(
    backend: web::Data<Arc<dyn NotificationBackend>>,
    payload: web::Json<Value>,
) -> HttpResponse {
    match handle_cloudevent(payload.into_inner()).await {
        Ok(response) => HttpResponse::Ok().json(response),
        Err(e) => validation_error_response("CloudEvent", e),
    }
}
