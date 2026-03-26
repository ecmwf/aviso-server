// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

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
