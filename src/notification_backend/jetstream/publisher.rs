use crate::notification_backend::jetstream::backend::JetStreamBackend;
use crate::telemetry::{SERVICE_NAME, SERVICE_VERSION};
use anyhow::{Context, Result, anyhow};
use async_nats::HeaderMap;
use std::collections::HashMap;
use std::time::Duration;
use tracing::{debug, info};

pub async fn put_messages(backend: &JetStreamBackend, topic: &str, payload: String) -> Result<()> {
    publish_with_retry(backend, topic, None, payload).await
}

/// Publish message with custom headers to JetStream
pub async fn put_message_with_headers(
    backend: &JetStreamBackend,
    topic: &str,
    headers: Option<HashMap<String, String>>,
    payload: String,
) -> Result<()> {
    publish_with_retry(backend, topic, headers, payload).await
}

async fn publish_with_retry(
    backend: &JetStreamBackend,
    topic: &str,
    headers: Option<HashMap<String, String>>,
    payload: String,
) -> Result<()> {
    let payload_size = payload.len();
    let max_attempts = backend.config.publish_retry_attempts;
    let base_backoff_ms = backend.config.publish_retry_base_delay_ms;
    for attempt in 1..=max_attempts {
        let result = publish_once(backend, topic, headers.as_ref(), &payload).await;
        match result {
            Ok(()) => return Ok(()),
            // Only retry transient transport failures; other publish errors are terminal.
            Err(error) if attempt < max_attempts && is_channel_closed_error(&error) => {
                let backoff_ms = base_backoff_ms.saturating_mul(1u64 << (attempt - 1));
                tracing::warn!(
                    service_name = SERVICE_NAME,
                    service_version = SERVICE_VERSION,
                    event_domain = "backend",
                    event_name = "backend.jetstream.publish.retry.channel_closed",
                    topic = %topic,
                    payload_size = payload_size,
                    has_headers = headers.is_some(),
                    attempt = attempt,
                    max_attempts = max_attempts,
                    backoff_ms = backoff_ms,
                    error = %error,
                    "JetStream publish failed with channel closed; retrying with backoff"
                );
                tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
            }
            Err(error) => return Err(error),
        }
    }
    // Defensive fallback: loop should have returned from the match above.
    Err(anyhow!(
        "publish_with_retry exhausted attempts without success or terminal error"
    ))
}

async fn publish_once(
    backend: &JetStreamBackend,
    topic: &str,
    headers: Option<&HashMap<String, String>>,
    payload: &str,
) -> Result<()> {
    let stream_name = match backend.ensure_stream_for_topic(topic).await {
        Ok(stream_name) => stream_name,
        Err(error) => {
            tracing::warn!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_domain = "backend",
                event_name = "backend.jetstream.ensure_stream.failed",
                topic = %topic,
                error = %error,
                "Failed to ensure stream for topic before publish"
            );
            return Err(error);
        }
    };

    debug!(
        topic = %topic,
        stream_name = %stream_name,
        payload_size = payload.len(),
        has_headers = headers.is_some(),
        "Publishing notification message to JetStream"
    );

    let publish_ack = if let Some(header_map) = headers {
        let jetstream_headers = build_jetstream_headers(header_map);
        backend
            .jetstream
            .publish_with_headers(
                topic.to_string(),
                jetstream_headers,
                payload.to_string().into(),
            )
            .await
            .context("Failed to publish notification message with headers to JetStream")?
    } else {
        backend
            .jetstream
            .publish(topic.to_string(), payload.to_string().into())
            .await
            .context("Failed to publish notification message to JetStream")?
    };

    let ack = publish_ack
        .await
        .context("Failed to receive publish acknowledgment from JetStream")?;
    let event_name = if headers.is_some() {
        "backend.jetstream.publish_with_headers.succeeded"
    } else {
        "backend.jetstream.publish.succeeded"
    };

    info!(
        service_name = SERVICE_NAME,
        service_version = SERVICE_VERSION,
        event_domain = "backend",
        event_name = event_name,
        topic = %topic,
        stream_name = %stream_name,
        sequence = ack.sequence,
        payload_size = payload.len(),
        "Notification message published successfully to JetStream"
    );

    Ok(())
}

fn is_channel_closed_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .to_string()
            .to_ascii_lowercase()
            .contains("channel closed")
    })
}

fn build_jetstream_headers(headers: &HashMap<String, String>) -> HeaderMap {
    let mut jetstream_headers = HeaderMap::new();
    for (key, value) in headers {
        jetstream_headers.insert(key.clone(), value.clone());
    }
    jetstream_headers
}
