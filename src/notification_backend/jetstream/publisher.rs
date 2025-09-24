use crate::notification_backend::jetstream::backend::JetStreamBackend;
use anyhow::{Context, Result};
use async_nats::HeaderMap;
use std::collections::HashMap;
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

/// Publish message with custom headers to JetStream
pub async fn put_message_with_headers(
    backend: &JetStreamBackend,
    topic: &str,
    headers: Option<HashMap<String, String>>,
    payload: String,
) -> Result<()> {
    // Ensure the appropriate stream exists for this topic
    let stream_name = backend.ensure_stream_for_topic(topic).await?;

    debug!(
        topic = %topic,
        stream_name = %stream_name,
        payload_size = payload.len(),
        has_headers = headers.is_some(),
        "Publishing notification message to JetStream with headers"
    );

    if let Some(header_map) = headers {
        // Convert HashMap to JetStream headers - much simpler than I thought!
        let mut jetstream_headers = HeaderMap::new();
        for (key, value) in header_map {
            jetstream_headers.insert(key, value);
        }

        // Publish with headers using backend.jetstream.publish_with_headers
        let publish_ack = backend
            .jetstream
            .publish_with_headers(topic.to_string(), jetstream_headers, payload.clone().into())
            .await
            .context("Failed to publish notification message with headers to JetStream")?;

        // Wait for acknowledgment from JetStream
        let ack = publish_ack
            .await
            .context("Failed to receive publish acknowledgment from JetStream")?;

        info!(
            topic = %topic,
            stream_name = %stream_name,
            sequence = ack.sequence,
            payload_size = payload.len(),
            "Notification message with headers published successfully to JetStream"
        );
    } else {
        // No headers, use regular publish
        put_messages(backend, topic, payload).await?;
    }

    Ok(())
}
