use crate::notification_backend::{NotificationBackend, NotificationMessage};
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures_util::Stream;
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

/// Configuration for in-memory backend
/// Contains all settings for memory limits, topic management, and monitoring
#[derive(Debug, Clone)]
pub struct InMemoryConfig {
    /// Maximum number of messages to keep per topic (older messages are discarded)
    pub max_history_per_topic: usize,
    /// Maximum number of topics to track (oldest topics are removed when limit is reached)
    pub max_topics: usize,
    /// Whether to enable detailed metrics logging for performance monitoring
    pub enable_metrics: bool,
}

impl InMemoryConfig {
    /// Create InMemoryConfig from application configuration
    /// Merges configuration file settings with sensible defaults
    /// All limits are designed to prevent unbounded memory growth
    ///
    /// # Arguments
    /// * `settings` - Application notification backend settings
    ///
    /// # Returns
    /// * `Self` - Configured in-memory backend settings with defaults applied
    pub fn from_backend_settings(
        settings: &crate::configuration::NotificationBackendSettings,
    ) -> Self {
        let in_memory_settings = settings.in_memory.as_ref();
        Self {
            max_history_per_topic: in_memory_settings
                .and_then(|im| im.max_history_per_topic)
                .unwrap_or(1), // Default to 1 message per topic (latest only)
            max_topics: in_memory_settings
                .and_then(|im| im.max_topics)
                .unwrap_or(10000), // Default to 10,000 topics maximum
            enable_metrics: in_memory_settings
                .and_then(|im| im.enable_metrics)
                .unwrap_or(false), // Default metrics disabled for performance
        }
    }
}

/// Internal state tracking for each topic
/// Maintains message history with automatic pruning and statistics
#[derive(Debug)]
struct TopicState {
    /// Queue of messages for this topic (oldest at front, newest at back)
    messages: VecDeque<NotificationMessage>,
    /// Next message ID to assign (monotonically increasing per topic)
    next_id: u64,
    /// Total number of messages ever received for this topic (for statistics)
    total_messages_received: u64,
}

impl TopicState {
    /// Create new topic state with pre-allocated capacity
    ///
    /// # Arguments
    /// * `capacity` - Initial capacity for the message queue to avoid reallocations
    ///
    /// # Returns
    /// * `Self` - New topic state ready to receive messages
    fn new(capacity: usize) -> Self {
        Self {
            messages: VecDeque::with_capacity(capacity),
            next_id: 1, // Start message IDs at 1
            total_messages_received: 0,
        }
    }
}

/// In-memory notification backend with configurable limits
/// Provides fast, volatile storage for notifications with automatic memory management
/// Data is lost when the application restarts, but offers excellent performance
/// Implements LRU-style eviction for both topics and messages within topics
#[derive(Clone)]
pub struct InMemoryBackend {
    /// Thread-safe storage mapping topic names to their state
    topics: Arc<Mutex<HashMap<String, TopicState>>>,
    /// Configuration controlling memory limits and behavior
    config: InMemoryConfig,
}

impl InMemoryBackend {
    /// Create a new in-memory backend with specified configuration
    /// Initializes empty storage with configured limits
    ///
    /// # Arguments
    /// * `config` - Configuration specifying memory limits and behavior
    ///
    /// # Returns
    /// * `Self` - Configured in-memory backend ready to store notifications
    pub fn new(config: InMemoryConfig) -> Self {
        info!(
            max_history_per_topic = config.max_history_per_topic,
            max_topics = config.max_topics,
            enable_metrics = config.enable_metrics,
            "Initializing in-memory backend with configuration"
        );

        Self {
            topics: Arc::new(Mutex::new(HashMap::new())),
            config,
        }
    }

    /// Get current statistics for monitoring and debugging
    /// Provides insight into memory usage and message distribution
    ///
    /// # Returns
    /// * `InMemoryStats` - Current backend statistics including counts and limits
    pub async fn get_stats(&self) -> InMemoryStats {
        let topics = self.topics.lock().await;
        let total_topics = topics.len();
        let total_messages: usize = topics.values().map(|state| state.messages.len()).sum();
        let total_received: u64 = topics
            .values()
            .map(|state| state.total_messages_received)
            .sum();

        InMemoryStats {
            total_topics,
            total_messages,
            total_messages_received: total_received,
            max_history_per_topic: self.config.max_history_per_topic,
            max_topics: self.config.max_topics,
        }
    }

    /// Enforce topic limit by removing the oldest topics when necessary
    /// Uses LRU strategy based on the timestamp of the most recent message per topic
    /// This prevents unbounded memory growth when many topics are created
    ///
    /// # Arguments
    /// * `topics` - Mutable reference to the topics HashMap for modification
    async fn enforce_topic_limit(&self, topics: &mut HashMap<String, TopicState>) {
        if topics.len() >= self.config.max_topics {
            // Find the topic with the oldest most recent message (LRU strategy)
            let oldest_topic = topics
                .iter()
                .min_by_key(|(_, state)| {
                    // Use the timestamp of the most recent message, or 0 if no messages
                    // Handle Option<i64> timestamp
                    state
                        .messages
                        .back()
                        .and_then(|msg| msg.timestamp) // Extract Option<i64>
                        .unwrap_or_else(|| Utc::now()) // Convert to i64 with default 0
                })
                .map(|(topic, _)| topic.clone());

            if let Some(topic_to_remove) = oldest_topic {
                topics.remove(&topic_to_remove);
                warn!(
                    removed_topic = %topic_to_remove,
                    max_topics = self.config.max_topics,
                    "Removed oldest topic due to topic limit enforcement"
                );
            }
        }
    }
}

/// Statistics about in-memory backend state
/// Provides visibility into current usage and configured limits
#[derive(Debug, Clone)]
pub struct InMemoryStats {
    /// Current number of active topics
    pub total_topics: usize,
    /// Current number of stored messages across all topics
    pub total_messages: usize,
    /// Total number of messages received since backend creation
    pub total_messages_received: u64,
    /// Configured maximum messages per topic
    pub max_history_per_topic: usize,
    /// Configured maximum number of topics
    pub max_topics: usize,
}

/// Implementation of NotificationBackend trait for in-memory storage
/// Provides fast, volatile notification storage with automatic memory management
/// All operations are lock-protected for thread safety
#[async_trait]
impl NotificationBackend for InMemoryBackend {
    /// Store a notification message for the specified topic
    /// Automatically manages memory limits by pruning old messages and topics
    /// Creates new topics on-demand and assigns unique message IDs per topic
    ///
    /// # Arguments
    /// * `topic` - Topic name to store the message under
    /// * `payload` - Notification payload as JSON string
    ///
    /// # Returns
    /// * `anyhow::Result<()>` - Success or error if timestamp generation fails
    async fn put_messages(&self, topic: &str, payload: String) -> Result<()> {
        let mut topics = self.topics.lock().await;

        // Enforce topic limit before potentially creating a new topic
        if !topics.contains_key(topic) {
            self.enforce_topic_limit(&mut topics).await;
        }

        // Get or create topic state with configured capacity
        let topic_state = topics.entry(topic.to_string()).or_insert_with(|| {
            info!(
                topic = %topic,
                max_history = self.config.max_history_per_topic,
                "Creating new topic with configured history limit"
            );
            TopicState::new(self.config.max_history_per_topic)
        });

        // Create message with optional fields populated for in-memory backend
        let msg = NotificationMessage {
            sequence: topic_state.next_id, // Remove Some(), it's already u64
            topic: topic.to_string(),      // Add missing topic field
            payload: payload.to_string(),  // Keep as String
            timestamp: Some(Utc::now()),   // Use DateTime<Utc>, not i64
        };

        // Update topic state counters
        topic_state.next_id += 1;
        topic_state.total_messages_received += 1;

        // Enforce per-topic message history limit (FIFO eviction)
        if topic_state.messages.len() >= self.config.max_history_per_topic {
            let removed_msg = topic_state.messages.pop_front();
            debug!(
                topic = %topic,
                removed_msg_id = removed_msg.as_ref().map(|m| m.sequence),
                max_history = self.config.max_history_per_topic,
                "Pruned oldest message due to history limit"
            );
        }

        // Add new message to the back of the queue
        topic_state.messages.push_back(msg);

        // Log with optional detailed metrics
        if self.config.enable_metrics {
            debug!(
                topic = %topic,
                msg_id = topic_state.next_id - 1,
                queue_size = topic_state.messages.len(),
                total_received = topic_state.total_messages_received,
                "Message stored with detailed metrics"
            );
        } else {
            debug!(
                topic = %topic,
                msg_id = topic_state.next_id - 1,
                "Message stored successfully"
            );
        }

        Ok(())
    }

    /// Remove all notifications from topics matching a stream pattern
    /// For in-memory backend, identifies stream topics by prefix matching
    /// Example: stream "diss" removes all topics starting with "diss."
    ///
    /// # Arguments
    /// * `stream_name` - Stream name to match against topic prefixes
    ///
    /// # Returns
    /// * `anyhow::Result<()>` - Always succeeds for in-memory backend
    async fn wipe_stream(&self, stream_name: &str) -> Result<()> {
        let mut topics = self.topics.lock().await;
        let stream_prefix = format!("{}.", stream_name.to_lowercase());

        // Collect all topic keys that match the stream prefix
        // Done separately to avoid borrowing issues during HashMap modification
        let keys_to_remove: Vec<String> = topics
            .keys()
            .filter(|key| key.to_lowercase().starts_with(&stream_prefix))
            .cloned()
            .collect();

        let mut removed_subjects = 0;
        let mut total_notifications = 0;

        // Remove all matching topics and count removed messages
        for key in keys_to_remove {
            if let Some(topic_state) = topics.remove(&key) {
                total_notifications += topic_state.messages.len();
                removed_subjects += 1;
                debug!(
                    topic = %key,
                    messages_removed = topic_state.messages.len(),
                    "Removed topic as part of stream wipe"
                );
            }
        }

        info!(
            stream_name = %stream_name,
            subjects_removed = removed_subjects,
            notifications_removed = total_notifications,
            "Wiped stream from in-memory backend"
        );

        Ok(())
    }

    /// Remove all notifications from all topics
    /// This is a complete reset operation that clears all stored data
    /// Use with caution as this operation cannot be undone
    ///
    /// # Returns
    /// * `anyhow::Result<()>` - Always succeeds for in-memory backend
    async fn wipe_all(&self) -> Result<()> {
        let mut topics = self.topics.lock().await;

        // Collect statistics before clearing for logging
        let subjects_count = topics.len();
        let total_notifications: usize = topics.values().map(|state| state.messages.len()).sum();

        // Clear all topics and their associated messages
        topics.clear();

        info!(
            subjects_removed = subjects_count,
            notifications_removed = total_notifications,
            "Wiped all data from in-memory backend - complete reset performed"
        );

        Ok(())
    }

    #[allow(unused_variables)]
    async fn get_messages_batch(
        &self,
        topic: &str,
        from_sequence: Option<u64>,
        from_date: Option<DateTime<Utc>>,
        limit: usize,
        offset: usize,
    ) -> anyhow::Result<(Vec<NotificationMessage>, bool)> {
        // TODO: Implement InMemory message retrieval
        todo!("InMemory get_messages_batch not yet implemented")
    }

    #[allow(unused_variables)]
    async fn count_messages(
        &self,
        topic: &str,
        from_sequence: Option<u64>,
        from_date: Option<DateTime<Utc>>,
    ) -> anyhow::Result<usize> {
        // TODO: Implement InMemory message counting
        todo!("InMemory count_messages not yet implemented")
    }

    #[allow(unused_variables)]
    async fn subscribe_to_topic(
        &self,
        topic: &str,
    ) -> anyhow::Result<Box<dyn Stream<Item = NotificationMessage> + Unpin + Send>> {
        // TODO: Implement InMemory real-time subscription
        todo!("InMemory subscribe_to_topic not yet implemented")
    }
}
