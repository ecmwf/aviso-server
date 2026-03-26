// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use crate::notification_backend::NotificationMessage;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Response structure for successful notification processing
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct NotificationResponse {
    pub status: String,
    pub request_id: String,
    pub processed_at: String,
}

/// Information about replay limiting applied during batch retrieval
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayLimitInfo {
    /// Maximum allowed messages from configuration
    pub max_allowed: usize,
}

/// Batch retrieval response for replay functionality
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchResult {
    /// Messages retrieved in this batch
    pub messages: Vec<NotificationMessage>,
    /// Whether more messages are available for pagination
    pub has_more: bool,
    /// Highest sequence number in this batch (for next batch starting point)
    pub last_sequence: Option<u64>,
    /// Next sequence number to request for efficient pagination
    pub next_sequence: Option<u64>,
    /// Total number of messages in this batch
    pub batch_size: usize,
    /// Replay limiting information if applied
    pub replay_limit: Option<ReplayLimitInfo>,
}

impl BatchResult {
    /// Create a new BatchResult with automatic pagination metadata calculation
    pub fn new(messages: Vec<NotificationMessage>, requested_limit: usize) -> Self {
        let batch_size = messages.len();
        let has_more = batch_size == requested_limit;
        let last_sequence = messages.iter().map(|msg| msg.sequence).max();
        let next_sequence = last_sequence.map(|seq| seq + 1);
        Self {
            messages,
            has_more,
            last_sequence,
            next_sequence,
            batch_size,
            replay_limit: None, // No limit by default
        }
    }

    /// Create an empty BatchResult indicating no more messages
    pub fn empty() -> Self {
        Self {
            messages: Vec::new(),
            has_more: false,
            last_sequence: None,
            next_sequence: None,
            batch_size: 0,
            replay_limit: None,
        }
    }
}
