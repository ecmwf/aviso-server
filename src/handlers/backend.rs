use crate::notification_backend::NotificationBackend;
use std::sync::Arc;
use tracing::info;

pub async fn save_to_backend(
    notification_result: &crate::notification::ProcessingResult,
    backend: &Arc<dyn NotificationBackend>,
) -> Result<(), anyhow::Error> {
    let payload_to_save = if let Some(payload_data) = &notification_result.payload {
        payload_data.clone()
    } else {
        serde_json::to_string(&notification_result.canonicalized_params)?
    };

    backend
        .put_messages(&notification_result.topic, payload_to_save)
        .await?;

    info!(
        event_type = %notification_result.event_type,
        topic = %notification_result.topic,
        "Notification saved to backend successfully"
    );

    Ok(())
}
