use chrono::{DateTime, Utc};

/// Parameters for batch message retrieval
///
/// Encapsulates all parameters needed for retrieving historical messages
/// in a backend-agnostic way.
#[derive(Debug, Clone)]
pub struct BatchParams {
    /// Topic pattern to retrieve messages for
    pub topic: String,
    /// Starting sequence number (takes precedence over from_date)
    pub from_sequence: Option<u64>,
    /// Starting timestamp for message retrieval
    pub from_date: Option<DateTime<Utc>>,
    /// Maximum number of messages to retrieve in this batch
    pub limit: usize,
}

impl BatchParams {
    pub fn new(topic: String, limit: usize) -> Self {
        Self {
            topic,
            from_sequence: None,
            from_date: None,
            limit,
        }
    }

    pub fn with_sequence(mut self, sequence: u64) -> Self {
        self.from_sequence = Some(sequence);
        self
    }

    pub fn with_date(mut self, date: DateTime<Utc>) -> Self {
        self.from_date = Some(date);
        self
    }
}
