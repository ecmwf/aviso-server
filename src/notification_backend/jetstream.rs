use crate::notification_backend::{NotificationBackend, NotificationMessage};
use anyhow::{Context, Result, bail};
use async_nats::jetstream::{
    self,
    stream::{Config as StreamConfig, DiscardPolicy, RetentionPolicy, StorageType},
};
use async_trait::async_trait;
use futures::StreamExt;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, info, warn};

/// Configuration for JetStream backend
/// Contains all necessary settings for connecting to NATS and configuring streams
#[derive(Debug, Clone)]
pub struct JetStreamConfig {
    /// NATS server URL (e.g., "nats://localhost:4222")
    pub nats_url: String,
    /// Connection timeout in seconds
    pub timeout_seconds: u64,
    /// Number of retry attempts for failed operations
    pub retry_attempts: u32,
    /// Optional authentication token for NATS
    pub token: Option<String>,
    /// Maximum number of messages per stream
    pub max_messages: Option<i64>,
    /// Maximum bytes per stream
    pub max_bytes: Option<i64>,
    /// Maximum age of messages in seconds
    pub max_age_seconds: Option<i64>,
    /// Storage type: "file" or "memory"
    pub storage_type: String,
    /// Number of replicas for high availability
    pub replicas: Option<usize>,
    /// Retention policy: "limits", "interest", or "workqueue"
    pub retention_policy: String,
    /// Discard policy when limits are reached: "old" or "new"
    pub discard_policy: String,
}

impl JetStreamConfig {
    /// Create JetStreamConfig from application configuration
    /// Merges configuration file settings
    /// Environment variables take precedence over config file values
    pub fn from_backend_settings(
        settings: &crate::configuration::NotificationBackendSettings,
    ) -> Self {
        let js_settings = settings.jetstream.as_ref();
        Self {
            nats_url: js_settings
                .and_then(|js| js.nats_url.clone())
                .unwrap_or_else(|| "nats://localhost:4222".to_string()),
            timeout_seconds: js_settings.and_then(|js| js.timeout_seconds).unwrap_or(30),
            retry_attempts: js_settings.and_then(|js| js.retry_attempts).unwrap_or(3),
            token: js_settings
                .and_then(|js| js.token.clone())
                .or_else(|| std::env::var("NATS_TOKEN").ok()),
            max_messages: js_settings.and_then(|js| js.max_messages),
            max_bytes: js_settings.and_then(|js| js.max_bytes),
            max_age_seconds: js_settings
                .and_then(|js| js.retention_days)
                .map(|days| days as i64 * 24 * 3600),
            storage_type: js_settings
                .and_then(|js| js.storage_type.clone())
                .unwrap_or_else(|| "file".to_string()),
            replicas: js_settings.and_then(|js| js.replicas),
            retention_policy: js_settings
                .and_then(|js| js.retention_policy.clone())
                .unwrap_or_else(|| "limits".to_string()),
            discard_policy: js_settings
                .and_then(|js| js.discard_policy.clone())
                .unwrap_or_else(|| "old".to_string()),
        }
    }
}

/// JetStream backend implementation
/// Manages multiple streams based on topic bases (e.g., "diss", "mars")
/// Each base topic gets its own stream with specific subject patterns
/// This prevents subject overlap and provides better isolation between different notification types
#[derive(Clone)]
pub struct JetStreamBackend {
    /// NATS client connection (kept alive for connection management)
    #[allow(dead_code)]
    client: async_nats::Client,
    /// JetStream context for stream operations
    jetstream: jetstream::Context,
    /// Configuration for this backend instance
    config: JetStreamConfig,
}

impl JetStreamBackend {
    /// Create a new JetStream backend
    /// This initializes the connection but doesn't create streams yet
    /// Streams are created on-demand when messages are published
    ///
    /// # Arguments
    /// * `config` - JetStream configuration containing connection details and stream settings
    ///
    /// # Returns
    /// * `Result<Self>` - Configured JetStream backend or error if connection fails
    pub async fn new(config: JetStreamConfig) -> Result<Self> {
        info!(
            nats_url = %config.nats_url,
            "Initializing JetStream backend with per-topic stream architecture"
        );

        // Establish connection to NATS server with optional token authentication
        let client = if let Some(token) = &config.token {
            info!("Connecting to NATS with token authentication");
            let connect_options = async_nats::ConnectOptions::new().token(token.clone());
            async_nats::connect_with_options(&config.nats_url, connect_options)
                .await
                .context("Failed to connect to NATS server with token authentication")?
        } else {
            info!("Connecting to NATS without authentication");
            async_nats::connect(&config.nats_url)
                .await
                .context("Failed to connect to NATS server")?
        };

        // Create JetStream context for stream operations
        let jetstream = jetstream::new(client.clone());

        let backend = Self {
            client,
            jetstream,
            config,
        };

        info!("JetStream backend initialized successfully - streams will be created on-demand");
        Ok(backend)
    }

    /// Ensure a stream exists for the given topic
    /// Creates streams on-demand based on topic base (e.g., "diss.foo.bar" -> "DISS" stream)
    /// This prevents subject overlap by creating separate streams for each base
    ///
    /// # Arguments
    /// * `topic` - Full topic name (e.g., "diss.FOO.E1.od.g.1.20190810.0.enfo.1")
    ///
    /// # Returns
    /// * `Result<String>` - Stream name that handles this topic or error if creation fails
    async fn ensure_stream_for_topic(&self, topic: &str) -> Result<String> {
        // Extract base from topic (first part before '.')
        let base = topic
            .split('.')
            .next()
            .ok_or_else(|| anyhow::anyhow!("Invalid topic format: {}", topic))?;

        // Create stream name by uppercasing the base
        let stream_name = base.to_uppercase();
        // Create subject pattern to match all topics with this base
        let subject_pattern = format!("{}.>", base);

        debug!(
            topic = %topic,
            base = %base,
            stream_name = %stream_name,
            subject_pattern = %subject_pattern,
            "Determining stream for topic"
        );

        // Check if stream already exists to avoid unnecessary creation attempts
        match self.jetstream.get_stream(&stream_name).await {
            Ok(_) => {
                debug!(stream_name = %stream_name, "Stream already exists");
                return Ok(stream_name);
            }
            Err(_) => {
                info!(
                    stream_name = %stream_name,
                    subject_pattern = %subject_pattern,
                    "Creating new stream for base topic"
                );
            }
        }

        // Create stream configuration for this specific base
        let stream_config = self.build_stream_config_for_base(&stream_name, &subject_pattern)?;

        // Attempt to create the stream with proper error handling
        match self.jetstream.create_stream(stream_config).await {
            Ok(_) => {
                info!(
                    stream_name = %stream_name,
                    subject_pattern = %subject_pattern,
                    "Stream created successfully"
                );
                Ok(stream_name)
            }
            Err(e) => {
                let error_msg = e.to_string();
                // Handle race condition where another replica creates the stream
                if error_msg.contains("stream name already in use") {
                    info!(stream_name = %stream_name, "Stream created by another replica");
                    Ok(stream_name)
                } else {
                    warn!(
                        stream_name = %stream_name,
                        subject_pattern = %subject_pattern,
                        error = %e,
                        "Failed to create stream"
                    );
                    Err(e.into())
                }
            }
        }
    }

    /// Build stream configuration for a specific base topic
    /// This creates the proper configuration with subject patterns and limits
    /// Each stream only handles messages for its specific base (e.g., "diss.>")
    ///
    /// # Arguments
    /// * `stream_name` - Name of the stream (e.g., "DISS")
    /// * `subject_pattern` - Subject pattern for this stream (e.g., "diss.>")
    ///
    /// # Returns
    /// * `Result<StreamConfig>` - Configured stream settings or error if invalid configuration
    fn build_stream_config_for_base(
        &self,
        stream_name: &str,
        subject_pattern: &str,
    ) -> Result<StreamConfig> {
        // Parse storage type from configuration
        let storage_type = match self.config.storage_type.to_lowercase().as_str() {
            "file" => StorageType::File,
            "memory" => StorageType::Memory,
            _ => bail!("Invalid storage type: '{}'", self.config.storage_type),
        };

        // Parse retention policy from configuration
        let retention = match self.config.retention_policy.to_lowercase().as_str() {
            "limits" => RetentionPolicy::Limits,
            "interest" => RetentionPolicy::Interest,
            "workqueue" => RetentionPolicy::WorkQueue,
            _ => bail!(
                "Invalid retention policy: '{}'",
                self.config.retention_policy
            ),
        };

        // Parse discard policy from configuration
        let discard = match self.config.discard_policy.to_lowercase().as_str() {
            "old" => DiscardPolicy::Old,
            "new" => DiscardPolicy::New,
            _ => bail!("Invalid discard policy: '{}'", self.config.discard_policy),
        };

        // Create base stream configuration with per-subject message limiting
        let mut config = StreamConfig {
            name: stream_name.to_string(),
            subjects: vec![subject_pattern.to_string()], // Only match this base's topics
            storage: storage_type,
            retention,
            discard,
            max_messages_per_subject: 1, // Keep only the latest message per subject
            ..Default::default()
        };

        // Apply optional limits from configuration
        if let Some(max_messages) = self.config.max_messages {
            config.max_messages = max_messages;
        }
        if let Some(max_bytes) = self.config.max_bytes {
            config.max_bytes = max_bytes;
        }
        if let Some(max_age_seconds) = self.config.max_age_seconds {
            config.max_age = std::time::Duration::from_secs(max_age_seconds as u64);
        }
        if let Some(replicas) = self.config.replicas {
            config.num_replicas = replicas;
        }

        debug!(
            stream_name = %stream_name,
            subject_pattern = %subject_pattern,
            storage = ?config.storage,
            retention = ?config.retention,
            max_messages_per_subject = config.max_messages_per_subject,
            "Built stream configuration with per-subject limit"
        );

        Ok(config)
    }
}

/// Implementation of NotificationBackend trait for JetStream
/// This handles the actual message publishing to appropriate streams
/// Each message goes to the stream corresponding to its topic base
/// Provides persistent, distributed notification storage with automatic stream management
#[async_trait]
impl NotificationBackend for JetStreamBackend {
    /// Publish a notification message to the appropriate stream based on topic
    /// Automatically creates streams on-demand and handles message serialization
    ///
    /// # Arguments
    /// * `topic` - Full topic name that determines which stream to use
    /// * `payload` - Notification payload as JSON string
    ///
    /// # Returns
    /// * `anyhow::Result<()>` - Success or error if publishing fails
    async fn put_messages(&self, topic: &str, payload: String) -> Result<()> {
        // Ensure the appropriate stream exists for this topic
        let stream_name = self.ensure_stream_for_topic(topic).await?;

        debug!(
            topic = %topic,
            stream_name = %stream_name,
            payload_size = payload.len(),
            "Publishing notification message to JetStream"
        );

        // Create notification message with metadata
        let message = NotificationMessage {
            id: 0, // Will be assigned by JetStream
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .context("Failed to get current timestamp")?
                .as_millis() as i64,
            payload,
        };

        // Serialize message to JSON for storage
        let message_json = serde_json::to_string(&message)
            .context("Failed to serialize notification message to JSON")?;

        // Publish message to JetStream - no headers needed!
        // JetStream will automatically discard old messages for this subject
        let publish_ack = self
            .jetstream
            .publish(topic.to_string(), message_json.into())
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
            payload_size = message.payload.len(),
            "Notification message published successfully to JetStream"
        );

        Ok(())
    }

    /// Remove all notifications from a specific stream
    /// This purges all messages in the stream but keeps the stream configuration intact
    /// The stream can continue to receive new messages after being wiped
    ///
    /// # Arguments
    /// * `stream_name` - Name of the stream to purge (e.g., "DISS", "MARS")
    ///
    /// # Returns
    /// * `anyhow::Result<()>` - Success or error if stream doesn't exist or purge fails
    async fn wipe_stream(&self, stream_name: &str) -> Result<()> {
        // Get the stream handle for the specified stream name
        let mut stream = self
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
    async fn wipe_all(&self) -> Result<()> {
        info!("Starting complete wipe of all JetStream data");

        // Get iterator over all streams in the JetStream context
        let mut streams = self.jetstream.streams();
        let mut total_streams_purged = 0;
        let mut total_messages_purged = 0;

        // Iterate through all streams and purge each one
        while let Some(stream_info) = streams.next().await {
            match stream_info {
                Ok(info) => {
                    let stream_name = &info.config.name;
                    let message_count = info.state.messages;

                    // Attempt to wipe this individual stream
                    match self.wipe_stream(stream_name).await {
                        Ok(_) => {
                            total_streams_purged += 1;
                            total_messages_purged += message_count;
                            info!(
                                stream = %stream_name,
                                messages = message_count,
                                "Successfully purged stream"
                            );
                        }
                        Err(e) => {
                            warn!(
                                stream = %stream_name,
                                error = %e,
                                "Failed to purge stream during wipe_all operation"
                            );
                        }
                    }
                }
                Err(e) => {
                    warn!(
                        error = %e,
                        "Failed to get stream info during wipe_all operation"
                    );
                }
            }
        }

        info!(
            streams_purged = total_streams_purged,
            messages_purged = total_messages_purged,
            "Completed wipe_all operation - all JetStream data removed"
        );

        Ok(())
    }
}
