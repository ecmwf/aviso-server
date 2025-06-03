use anyhow::Result;
use async_trait::async_trait;
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::SystemTime,
};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use crate::notification_backend::{NotificationBackend, NotificationMessage};

const MAX_HISTORY: usize = 1000;

#[derive(Debug)]
struct TopicState {
    messages: VecDeque<NotificationMessage>,
    next_id: u64,
}

#[derive(Clone)]
pub struct InMemoryBackend {
    topics: Arc<Mutex<HashMap<String, TopicState>>>,
}

impl InMemoryBackend {
    pub fn new() -> Self {
        info!("Initializing in-memory backend");
        Self {
            topics: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl NotificationBackend for InMemoryBackend {
    async fn put_messages(&self, topic: &str, payload: String) -> Result<()> {
        let mut topics = self.topics.lock().await;
        let topic_state = topics.entry(topic.to_string()).or_insert_with(|| {
            info!(topic = %topic, "Creating new topic");
            TopicState {
                messages: VecDeque::with_capacity(MAX_HISTORY),
                next_id: 1,
            }
        });

        let msg = NotificationMessage {
            id: topic_state.next_id,
            timestamp: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)?
                .as_millis() as i64,
            payload,
        };

        topic_state.next_id += 1;

        if topic_state.messages.len() >= MAX_HISTORY {
            debug!(topic = %topic, "Pruning oldest message");
            topic_state.messages.pop_front();
        }
        topic_state.messages.push_back(msg);

        debug!(topic = %topic, msg_id = topic_state.next_id - 1, "Message stored");
        Ok(())
    }

    async fn get_messages(
        &self,
        topic: &str,
        since_id: Option<u64>,
        since_timestamp: Option<i64>,
        limit: usize,
    ) -> Result<Vec<NotificationMessage>> {
        let topics = self.topics.lock().await;
        let topic_state = match topics.get(topic) {
            Some(state) => state,
            None => {
                warn!(topic = %topic, "Topic not found");
                return Ok(vec![]);
            }
        };

        let messages: Vec<_> = topic_state
            .messages
            .iter()
            .filter(|msg| {
                since_id.map(|id| msg.id >= id).unwrap_or(true)
                    && since_timestamp
                        .map(|ts| msg.timestamp >= ts)
                        .unwrap_or(true)
            })
            .take(limit)
            .cloned()
            .collect();

        debug!(topic = %topic, count = messages.len(), "Messages retrieved");
        Ok(messages)
    }
}
