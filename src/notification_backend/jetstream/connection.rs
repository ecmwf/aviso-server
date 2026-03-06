use super::config::JetStreamConfig;
use anyhow::{Context, Result};
use std::time::Duration;
use tracing::{info, warn};

use crate::notification_backend::jetstream::backend::JetStreamBackend;
use crate::telemetry::{SERVICE_NAME, SERVICE_VERSION};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ConnectionPolicy {
    initial_connect_attempts: u32,
    max_reconnects: Option<usize>,
    reconnect_delay_ms: u64,
}

fn build_connection_policy(config: &JetStreamConfig) -> ConnectionPolicy {
    let initial_connect_attempts = config.retry_attempts.max(1);

    let max_reconnects = if config.enable_auto_reconnect {
        if config.max_reconnect_attempts == 0 {
            None
        } else {
            Some(config.max_reconnect_attempts as usize)
        }
    } else {
        Some(0)
    };

    ConnectionPolicy {
        initial_connect_attempts,
        max_reconnects,
        reconnect_delay_ms: config.reconnect_delay_ms,
    }
}

fn build_connect_options(
    config: &JetStreamConfig,
    policy: ConnectionPolicy,
) -> async_nats::ConnectOptions {
    let mut options = async_nats::ConnectOptions::new()
        .connection_timeout(Duration::from_secs(config.timeout_seconds))
        .max_reconnects(policy.max_reconnects)
        .reconnect_delay_callback(move |_| Duration::from_millis(policy.reconnect_delay_ms));

    if let Some(token) = &config.token {
        options = options.token(token.clone());
    }

    options
}

pub async fn connect(config: JetStreamConfig) -> Result<JetStreamBackend> {
    info!(
        service_name = SERVICE_NAME,
        service_version = SERVICE_VERSION,
        event_name = "backend.jetstream.connection.started",
        url = %config.nats_url,
        "Connecting to NATS"
    );

    let policy = build_connection_policy(&config);
    let mut connected_client = None;
    for attempt in 1..=policy.initial_connect_attempts {
        let options = build_connect_options(&config, policy);
        match async_nats::connect_with_options(&config.nats_url, options).await {
            Ok(client) => {
                connected_client = Some(client);
                break;
            }
            Err(error) => {
                if attempt >= policy.initial_connect_attempts {
                    return Err(error).context("NATS connect failed");
                }

                warn!(
                    service_name = SERVICE_NAME,
                    service_version = SERVICE_VERSION,
                    event_name = "backend.jetstream.connection.retry",
                    url = %config.nats_url,
                    attempt = attempt,
                    max_attempts = policy.initial_connect_attempts,
                    retry_delay_ms = policy.reconnect_delay_ms,
                    error = %error,
                    "NATS connect attempt failed, retrying"
                );

                tokio::time::sleep(Duration::from_millis(policy.reconnect_delay_ms)).await;
            }
        }
    }

    let client = connected_client.context("NATS connect failed without a concrete error")?;

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

#[cfg(test)]
mod tests {
    use super::build_connection_policy;
    use crate::configuration::{
        JetStreamDiscardPolicy, JetStreamRetentionPolicy, JetStreamStorageType,
    };
    use crate::notification_backend::jetstream::config::JetStreamConfig;

    fn base_config() -> JetStreamConfig {
        JetStreamConfig {
            nats_url: "nats://localhost:4222".to_string(),
            timeout_seconds: 30,
            retry_attempts: 3,
            token: None,
            max_messages: None,
            max_bytes: None,
            retention_time: None,
            storage_type: JetStreamStorageType::File,
            replicas: None,
            retention_policy: JetStreamRetentionPolicy::Limits,
            discard_policy: JetStreamDiscardPolicy::Old,
            enable_auto_reconnect: true,
            max_reconnect_attempts: 5,
            reconnect_delay_ms: 2000,
            publish_retry_attempts: 5,
            publish_retry_base_delay_ms: 150,
        }
    }

    #[test]
    fn policy_enforces_minimum_single_initial_connect_attempt() {
        let mut cfg = base_config();
        cfg.retry_attempts = 0;
        let policy = build_connection_policy(&cfg);

        assert_eq!(policy.initial_connect_attempts, 1);
    }

    #[test]
    fn policy_disables_reconnect_when_auto_reconnect_is_false() {
        let mut cfg = base_config();
        cfg.enable_auto_reconnect = false;
        let policy = build_connection_policy(&cfg);

        assert_eq!(policy.max_reconnects, Some(0));
    }

    #[test]
    fn policy_uses_unlimited_reconnects_when_enabled_and_max_is_zero() {
        let mut cfg = base_config();
        cfg.max_reconnect_attempts = 0;
        let policy = build_connection_policy(&cfg);

        assert_eq!(policy.max_reconnects, None);
    }
}
