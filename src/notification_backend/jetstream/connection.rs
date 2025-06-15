use super::config::JetStreamConfig;
use anyhow::{Context, Result};
use tracing::info;

use crate::notification_backend::jetstream::backend::JetStreamBackend;


pub async fn connect(config: JetStreamConfig) -> Result<JetStreamBackend> {
    info!(url = %config.nats_url, "Connecting to NATS");
    let client = if let Some(token) = &config.token {
        let opts = async_nats::ConnectOptions::new().token(token.clone());
        async_nats::connect_with_options(&config.nats_url, opts)
            .await
            .context("NATS token connect failed")?
    } else {
        async_nats::connect(&config.nats_url)
            .await
            .context("NATS connect failed")?
    };

    let jetstream = async_nats::jetstream::new(client.clone());

    Ok(JetStreamBackend {
        client,
        jetstream,
        config,
    })
}

pub async fn shutdown(backend: &JetStreamBackend) -> Result<()> {
    backend.client.flush().await?;
    backend.client.drain().await?;
    Ok(())
}
