// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use crate::configuration::NotificationBackendSettings;

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
    pub fn from_backend_settings(settings: &NotificationBackendSettings) -> Self {
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
