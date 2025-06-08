use crate::notification::ProcessingResult;
use crate::notification_backend::NotificationBackend;
use anyhow::Result;
use tracing::{debug, info};

/// Save notification result to the configured backend
///
/// Takes a processed notification result and persists it to the backend storage.
/// The result contains the validated topic, canonicalized parameters, and payload.
#[tracing::instrument(
    skip(notification_backend),
    fields(
        topic = %result.topic,
        event_type = %result.event_type,
    )
)]
pub async fn save_to_backend(
    result: &ProcessingResult,
    payload: Option<&str>,
    notification_backend: &dyn NotificationBackend,
) -> Result<()> {
    debug!(
        topic = %result.topic,
        event_type = %result.event_type,
        param_count = result.canonicalized_params.len(),
        "Saving notification to backend"
    );

    // Extract payload or use empty string if None, converting to owned String
    let payload_str = payload.unwrap_or("");

    // Save the notification result to backend using put_messages
    notification_backend
        .put_messages(&result.topic, payload_str.parse()?)
        .await?;

    info!(
        topic = %result.topic,
        event_type = %result.event_type,
        "Notification saved to backend successfully"
    );

    Ok(())
}
