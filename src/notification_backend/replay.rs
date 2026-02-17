use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Copy)]
pub enum StartAt {
    /// No historical replay cursor. For watch this means live-only start.
    LiveOnly,
    /// Replay starting from an inclusive backend sequence.
    Sequence(u64),
    /// Replay starting from an inclusive UTC timestamp.
    Date(DateTime<Utc>),
}

/// Parameters for batch message retrieval
///
/// Encapsulates all parameters needed for retrieving historical messages
/// in a backend-agnostic way.
#[derive(Debug, Clone)]
pub struct BatchParams {
    /// Topic pattern to retrieve messages for
    pub topic: String,
    /// Starting cursor for historical replay retrieval.
    pub start_at: StartAt,
    /// Maximum number of messages to retrieve in this batch
    pub limit: usize,
}

impl BatchParams {
    pub fn new(topic: String, limit: usize) -> Self {
        Self {
            topic,
            start_at: StartAt::LiveOnly,
            limit,
        }
    }

    pub fn with_sequence(mut self, sequence: u64) -> Self {
        self.start_at = StartAt::Sequence(sequence);
        self
    }

    pub fn with_date(mut self, date: DateTime<Utc>) -> Self {
        self.start_at = StartAt::Date(date);
        self
    }

    pub fn with_start_at(mut self, start_at: StartAt) -> Self {
        self.start_at = start_at;
        self
    }
}

impl StartAt {
    /// Convert replay start selection into wire control fields.
    pub fn as_replay_cursor(self) -> (Option<u64>, Option<DateTime<Utc>>) {
        match self {
            StartAt::Sequence(seq) => (Some(seq), None),
            StartAt::Date(date) => (None, Some(date)),
            StartAt::LiveOnly => (None, None),
        }
    }
}
