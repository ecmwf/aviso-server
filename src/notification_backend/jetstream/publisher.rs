use crate::notification_backend::jetstream::backend::JetStreamBackend;
use anyhow::{Context, Result};
use tracing::{debug, info};

pub async fn put_messages(backend: &JetStreamBackend, topic: &str, payload: String) -> Result<()> {
    // Ensure the appropriate stream exists for this topic
    let stream_name = backend.ensure_stream_for_topic(topic).await?;

    debug!(
        topic = %topic,
        stream_name = %stream_name,
        payload_size = payload.len(),
        "Publishing notification message to JetStream"
    );

    // Publish raw payload directly to JetStream - no JSON wrapper
    // JetStream will provide its own sequence numbers and timestamps
    let publish_ack = backend
        .jetstream
        .publish(topic.to_string(), payload.clone().into())
        .await
        .context("Failed to publish notification message to JetStream")?;

    // Wait for acknowledgment from JetStream
    let ack = publish_ack
        .await
        .context("Failed to receive publish acknowledgment from JetStream")?;

    info!(
        topic = %topic,
        stream_name = %stream_name,
        sequence = ack.sequence,
        payload_size = payload.len(),
        "Notification message published successfully to JetStream"
    );

    Ok(())
}
