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
