use crate::notification_backend::NotificationBackend;
use crate::notification_backend::jetstream::backend::JetStreamBackend;
use crate::telemetry::{SERVICE_NAME, SERVICE_VERSION};
use anyhow::{Context, Result};
use futures::StreamExt;
use tracing::{info, warn};

/// Remove all notifications from a specific stream
/// This purges all messages in the stream but keeps the stream configuration intact
/// The stream can continue to receive new messages after being wiped
///
/// # Arguments
/// * `stream_name` - Name of the stream to purge (e.g., "DISS", "MARS")
///
/// # Returns
/// * `anyhow::Result<()>` - Success or error if stream doesn't exist or purge fails
pub async fn wipe_stream(backend: &JetStreamBackend, stream_name: &str) -> Result<()> {
    // Get the stream handle for the specified stream name
    let mut stream = backend
        .jetstream
        .get_stream(stream_name)
        .await
        .context(format!("Failed to get stream {}", stream_name))?;

    // Get current stream statistics before purging for logging
    let info = stream.info().await.context("Failed to get stream info")?;
    let total_messages = info.state.messages;

    // Purge all messages from the stream
    stream.purge().await.context("Failed to purge stream")?;

    info!(
        service_name = SERVICE_NAME,
        service_version = SERVICE_VERSION,
        event_domain = "admin",
        event_name = "admin.stream.wipe.succeeded",
        stream_name = %stream_name,
        messages_purged = total_messages,
        "Wiped entire stream - all messages removed but stream configuration preserved"
    );

    Ok(())
}

/// Remove all notifications from all streams in the JetStream context
/// This is a complete data reset operation that purges every stream
/// Stream configurations are preserved, only message data is removed
/// Use with caution as this operation cannot be undone
///
/// # Returns
/// * `anyhow::Result<()>` - Success or error if stream doesn't exist or purge fails
pub async fn wipe_all(backend: &JetStreamBackend) -> Result<()> {
    info!(
        service_name = SERVICE_NAME,
        service_version = SERVICE_VERSION,
        event_domain = "admin",
        event_name = "admin.all.wipe.started",
        "Starting complete wipe of all JetStream data"
    );

    // Get iterator over all streams in the JetStream context
    let mut streams = backend.jetstream.streams();
    let mut total_streams_purged = 0;
    let mut total_messages_purged = 0;

    // Iterate through all streams and purge each one
    while let Some(stream_info) = streams.next().await {
        match stream_info {
            Ok(info) => {
                let stream_name = &info.config.name;
                let message_count = info.state.messages;

                // Attempt to wipe this individual stream
                match backend.wipe_stream(stream_name).await {
                    Ok(_) => {
                        total_streams_purged += 1;
                        total_messages_purged += message_count;
                        info!(
                            service_name = SERVICE_NAME,
                            service_version = SERVICE_VERSION,
                            event_domain = "admin",
                            event_name = "admin.stream.wipe.succeeded",
                            stream = %stream_name,
                            messages = message_count,
                            "Successfully purged stream"
                        );
                    }
                    Err(e) => {
                        warn!(
                            service_name = SERVICE_NAME,
                            service_version = SERVICE_VERSION,
                            event_domain = "admin",
                            event_name = "admin.stream.wipe.failed",
                            stream = %stream_name,
                            error = %e,
                            "Failed to purge stream during wipe_all operation"
                        );
                    }
                }
            }
            Err(e) => {
                warn!(
                    service_name = SERVICE_NAME,
                    service_version = SERVICE_VERSION,
                    event_domain = "admin",
                    event_name = "admin.stream.info.failed",
                    error = %e,
                    "Failed to get stream info during wipe_all operation"
                );
            }
        }
    }

    info!(
        service_name = SERVICE_NAME,
        service_version = SERVICE_VERSION,
        event_domain = "admin",
        event_name = "admin.all.wipe.completed",
        streams_purged = total_streams_purged,
        messages_purged = total_messages_purged,
        "Completed wipe_all operation - all JetStream data removed"
    );

    Ok(())
}
