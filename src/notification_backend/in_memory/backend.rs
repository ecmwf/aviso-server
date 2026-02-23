use crate::notification::topic_codec::encode_token;
use crate::notification::wildcard_matcher::{analyze_watch_pattern, matches_watch_pattern};
use crate::notification_backend::in_memory::InMemoryConfig;
use crate::notification_backend::in_memory::InMemoryStats;
use crate::notification_backend::replay::{BatchParams, StartAt};
use crate::notification_backend::{NotificationBackend, NotificationMessage};
use crate::telemetry::{SERVICE_NAME, SERVICE_VERSION};
use crate::types::BatchResult;
use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use futures_util::Stream;
use futures_util::stream::unfold;
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};
use tokio::sync::{Mutex, broadcast};
use tracing::{debug, info, warn};

/// Internal state tracking for each topic
/// Maintains message history with automatic pruning and statistics
#[derive(Debug)]
struct TopicState {
    /// Queue of messages for this topic (oldest at front, newest at back)
    messages: VecDeque<NotificationMessage>,
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
            total_messages_received: 0,
        }
    }
}

#[derive(Debug)]
struct BackendState {
    topics: HashMap<String, TopicState>,
    next_sequence: u64,
}

/// In-memory notification backend with configurable limits
/// Provides fast, volatile storage for notifications with automatic memory management
/// Data is lost when the application restarts, but offers excellent performance
/// Implements LRU-style eviction for both topics and messages within topics
#[derive(Clone)]
pub struct InMemoryBackend {
    /// Thread-safe storage mapping topic names to their state
    state: Arc<Mutex<BackendState>>,
    /// Live notification fanout for subscriptions
    live_notifications_tx: broadcast::Sender<NotificationMessage>,
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
            service_name = SERVICE_NAME,
            service_version = SERVICE_VERSION,
            event_domain = "backend",
            event_name = "backend.in_memory.initialization.started",
            max_history_per_topic = config.max_history_per_topic,
            max_topics = config.max_topics,
            enable_metrics = config.enable_metrics,
            "Initializing in-memory backend with configuration"
        );

        let requested_channel_capacity = config
            .max_history_per_topic
            .saturating_mul(config.max_topics);
        let channel_capacity = requested_channel_capacity.clamp(1024, 65536);
        if requested_channel_capacity > channel_capacity {
            warn!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_domain = "backend",
                event_name = "backend.in_memory.channel.capacity.clamped",
                requested_channel_capacity,
                effective_channel_capacity = channel_capacity,
                "Broadcast channel capacity clamped to upper bound; lagged consumers may miss notifications under high throughput"
            );
        }
        let (live_notifications_tx, _) = broadcast::channel(channel_capacity);

        Self {
            state: Arc::new(Mutex::new(BackendState {
                topics: HashMap::new(),
                next_sequence: 1,
            })),
            live_notifications_tx,
            config,
        }
    }

    /// Get current statistics for monitoring and debugging
    /// Provides insight into memory usage and message distribution
    ///
    /// # Returns
    /// * `InMemoryStats` - Current backend statistics including counts and limits
    pub async fn get_stats(&self) -> InMemoryStats {
        let state = self.state.lock().await;
        let total_topics = state.topics.len();
        let total_messages: usize = state
            .topics
            .values()
            .map(|state| state.messages.len())
            .sum();
        let total_received: u64 = state
            .topics
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
    fn enforce_topic_limit(&self, topics: &mut HashMap<String, TopicState>) {
        if topics.len() >= self.config.max_topics {
            // Find the topic with the oldest most recent message (LRU strategy)
            let oldest_topic = topics
                .iter()
                .min_by_key(|(_, state)| {
                    state
                        .messages
                        .back()
                        .and_then(|msg| msg.timestamp)
                        .unwrap_or_else(Utc::now)
                })
                .map(|(topic, _)| topic.clone());

            if let Some(topic_to_remove) = oldest_topic {
                topics.remove(&topic_to_remove);
                warn!(
                    service_name = SERVICE_NAME,
                    service_version = SERVICE_VERSION,
                    event_domain = "backend",
                    event_name = "backend.in_memory.topic.evicted",
                    removed_topic = %topic_to_remove,
                    max_topics = self.config.max_topics,
                    "Removed oldest topic due to topic limit enforcement"
                );
            }
        }
    }

    async fn store_message(
        &self,
        topic: &str,
        payload: String,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<()> {
        let mut state = self.state.lock().await;
        let sequence = state.next_sequence;
        state.next_sequence += 1;

        if !state.topics.contains_key(topic) {
            self.enforce_topic_limit(&mut state.topics);
        }

        let topic_state = state.topics.entry(topic.to_string()).or_insert_with(|| {
            info!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_domain = "backend",
                event_name = "backend.in_memory.topic.created",
                topic = %topic,
                max_history = self.config.max_history_per_topic,
                "Creating new topic with configured history limit"
            );
            TopicState::new(self.config.max_history_per_topic)
        });

        let msg = NotificationMessage {
            sequence,
            topic: topic.to_string(),
            payload,
            timestamp: Some(Utc::now()),
            metadata,
        };

        topic_state.total_messages_received += 1;
        if topic_state.messages.len() >= self.config.max_history_per_topic {
            let removed_msg = topic_state.messages.pop_front();
            debug!(
                topic = %topic,
                removed_msg_id = removed_msg.as_ref().map(|m| m.sequence),
                max_history = self.config.max_history_per_topic,
                "Pruned oldest message due to history limit"
            );
        }

        topic_state.messages.push_back(msg.clone());

        if self.config.enable_metrics {
            debug!(
                topic = %topic,
                msg_id = msg.sequence,
                queue_size = topic_state.messages.len(),
                total_received = topic_state.total_messages_received,
                "Message stored with detailed metrics"
            );
        } else {
            debug!(
                topic = %topic,
                msg_id = msg.sequence,
                "Message stored successfully"
            );
        }

        let _ = self.live_notifications_tx.send(msg);
        Ok(())
    }
}

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
        self.store_message(topic, payload, None).await
    }

    async fn put_message_with_headers(
        &self,
        topic: &str,
        headers: Option<HashMap<String, String>>,
        payload: String,
    ) -> Result<()> {
        self.store_message(topic, payload, headers).await
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
        let mut state = self.state.lock().await;
        let topics = &mut state.topics;
        let stream_prefix = format!("{}.", encode_token(&stream_name.to_lowercase()));

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
            service_name = SERVICE_NAME,
            service_version = SERVICE_VERSION,
            event_domain = "backend",
            event_name = "backend.in_memory.stream.wiped",
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
        let mut state = self.state.lock().await;
        let topics = &mut state.topics;

        // Collect statistics before clearing for logging
        let subjects_count = topics.len();
        let total_notifications: usize = topics.values().map(|state| state.messages.len()).sum();

        // Clear all topics and their associated messages
        topics.clear();

        info!(
            service_name = SERVICE_NAME,
            service_version = SERVICE_VERSION,
            event_domain = "backend",
            event_name = "backend.in_memory.storage.wiped",
            subjects_removed = subjects_count,
            notifications_removed = total_notifications,
            "Wiped all data from in-memory backend - complete reset performed"
        );

        Ok(())
    }

    async fn get_messages_batch(&self, params: BatchParams) -> Result<BatchResult> {
        let (_backend_pattern, app_filter_pattern) = analyze_watch_pattern(&params.topic)?;

        let mut messages = {
            let state = self.state.lock().await;
            state
                .topics
                .values()
                .flat_map(|topic_state| topic_state.messages.iter())
                .filter(|message| matches_watch_pattern(&message.topic, &app_filter_pattern))
                .cloned()
                .collect::<Vec<_>>()
        };

        match params.start_at {
            StartAt::Sequence(from_sequence) if from_sequence > 0 => {
                messages.retain(|m| m.sequence >= from_sequence);
            }
            StartAt::Date(from_date) => {
                messages.retain(|m| m.timestamp.is_some_and(|ts| ts >= from_date));
            }
            StartAt::LiveOnly | StartAt::Sequence(_) => {}
        }

        if messages.is_empty() {
            return Ok(BatchResult::empty());
        }

        messages.sort_by_key(|m| m.sequence);

        let requested_limit = params.limit;
        let available_before_truncate = messages.len();
        messages.truncate(requested_limit);

        let mut result = BatchResult::new(messages, requested_limit);
        result.has_more = available_before_truncate > requested_limit;
        result.next_sequence = result.last_sequence.map(|seq| seq + 1);
        Ok(result)
    }

    async fn subscribe_to_topic(
        &self,
        topic: &str,
    ) -> anyhow::Result<Box<dyn Stream<Item = NotificationMessage> + Unpin + Send>> {
        let receiver = self.live_notifications_tx.subscribe();
        let (_backend_pattern, app_filter_pattern) = analyze_watch_pattern(topic)?;

        let stream = unfold(
            (receiver, app_filter_pattern),
            |(mut receiver, app_filter_pattern)| async move {
                loop {
                    match receiver.recv().await {
                        Ok(message) => {
                            if matches_watch_pattern(&message.topic, &app_filter_pattern) {
                                return Some((message, (receiver, app_filter_pattern)));
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            warn!(
                                service_name = SERVICE_NAME,
                                service_version = SERVICE_VERSION,
                                event_domain = "backend",
                                event_name = "backend.in_memory.subscription.lagged",
                                skipped = skipped,
                                "In-memory subscription lagged; dropped notifications"
                            );
                        }
                        Err(broadcast::error::RecvError::Closed) => return None,
                    }
                }
            },
        );

        Ok(Box::new(Box::pin(stream)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;
    use std::collections::HashMap;
    use tokio::time::{Duration, timeout};

    fn test_backend() -> InMemoryBackend {
        InMemoryBackend::new(InMemoryConfig {
            max_history_per_topic: 10,
            max_topics: 10,
            enable_metrics: false,
        })
    }

    #[tokio::test]
    async fn batch_replay_filters_by_sequence() {
        let backend = test_backend();
        backend
            .put_messages("mars.a", "one".to_string())
            .await
            .unwrap();
        backend
            .put_messages("mars.a", "two".to_string())
            .await
            .unwrap();

        let batch = backend
            .get_messages_batch(BatchParams {
                topic: "mars.a".to_string(),
                start_at: StartAt::Sequence(2),
                limit: 10,
            })
            .await
            .unwrap();

        assert_eq!(batch.messages.len(), 1);
        assert_eq!(batch.messages[0].payload, "two");
    }

    #[tokio::test]
    async fn batch_replay_filters_by_wildcard_topic() {
        let backend = test_backend();
        backend
            .put_messages("mars.a.1", "first".to_string())
            .await
            .unwrap();
        backend
            .put_messages("mars.b.1", "second".to_string())
            .await
            .unwrap();

        let batch = backend
            .get_messages_batch(BatchParams {
                topic: "mars.*.1".to_string(),
                start_at: StartAt::LiveOnly,
                limit: 10,
            })
            .await
            .unwrap();

        assert_eq!(batch.messages.len(), 2);
    }

    #[tokio::test]
    async fn batch_replay_filters_by_from_date() {
        let backend = test_backend();
        backend
            .put_messages("mars.time", "early".to_string())
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(5)).await;
        let boundary = Utc::now();
        tokio::time::sleep(Duration::from_millis(5)).await;
        backend
            .put_messages("mars.time", "late".to_string())
            .await
            .unwrap();

        let batch = backend
            .get_messages_batch(BatchParams {
                topic: "mars.time".to_string(),
                start_at: StartAt::Date(boundary),
                limit: 10,
            })
            .await
            .unwrap();

        assert_eq!(batch.messages.len(), 1);
        assert_eq!(batch.messages[0].payload, "late");
    }

    #[tokio::test]
    async fn subscription_is_live_only_and_preserves_headers() {
        let backend = test_backend();
        backend
            .put_messages("mars.live", "historical".to_string())
            .await
            .unwrap();

        let mut stream = backend.subscribe_to_topic("mars.live").await.unwrap();

        let mut headers = HashMap::new();
        headers.insert("spatial_bbox".to_string(), "1,1,2,2".to_string());

        backend
            .put_message_with_headers("mars.live", Some(headers), "live".to_string())
            .await
            .unwrap();

        let next = timeout(Duration::from_secs(2), stream.next())
            .await
            .expect("timed out waiting for live notification")
            .expect("stream ended unexpectedly");

        assert_eq!(next.payload, "live");
        assert!(
            next.metadata
                .as_ref()
                .and_then(|m| m.get("spatial_bbox"))
                .is_some()
        );
    }
}
