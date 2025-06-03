use crate::notification::{self, OperationType};
use actix_web::web;
use serde_json::Value;
use tracing::info;

/// Process Aviso notification using the notification module
pub async fn process_aviso(
    payload: &web::Json<Value>,
    operation: OperationType,
) -> Result<(), anyhow::Error> {
    let processing_result = notification::handler::extract_aviso_notification(payload, operation)?;

    info!(
        operation = ?operation,
        event_type = processing_result.event_type,
        topic = %processing_result.topic,
        param_count = processing_result.canonicalized_params.len(),
        "Aviso notification processed successfully"
    );

    Ok(())
}
