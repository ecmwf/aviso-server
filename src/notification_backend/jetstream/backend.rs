// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use crate::notification_backend::jetstream::{
    admin, connection, publisher, replay, streams, subscriber,
};
use crate::notification_backend::replay::BatchParams;
use crate::notification_backend::{
    BackendCapabilities, DeleteMessageResult, JETSTREAM_CAPABILITIES, NotificationBackend,
    Subscription, WipeStreamResult,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use std::collections::HashMap;

#[derive(Clone)]
pub struct JetStreamBackend {
    pub client: async_nats::Client,
    pub jetstream: async_nats::jetstream::Context,
    pub config: super::config::JetStreamConfig,
}

impl JetStreamBackend {
    pub async fn new(config: super::config::JetStreamConfig) -> Result<Self> {
        connection::connect(config).await
    }

    /* internal helpers simply delegate */
    pub async fn ensure_stream_for_topic(&self, topic: &str) -> Result<String> {
        streams::ensure_stream_for_topic(self, topic).await
    }
}

/* --------------------------------------------------------- */
/*   NotificationBackend trait – each method forwards        */
/* --------------------------------------------------------- */
#[async_trait]
impl NotificationBackend for JetStreamBackend {
    fn capabilities(&self) -> BackendCapabilities {
        JETSTREAM_CAPABILITIES
    }

    fn connection_healthy(&self) -> bool {
        // Pending (reconnecting) and Disconnected both mean requests
        // against the backend will fail right now; only a live connection
        // counts as ready.
        self.client.connection_state() == async_nats::connection::State::Connected
    }

    async fn put_messages(&self, topic: &str, payload: String) -> Result<()> {
        publisher::put_messages(self, topic, payload).await
    }

    async fn put_message_with_headers(
        &self,
        topic: &str,
        headers: Option<HashMap<String, String>>,
        payload: String,
    ) -> Result<()> {
        publisher::put_message_with_headers(self, topic, headers, payload).await
    }

    async fn wipe_stream(&self, stream_key: &str) -> Result<WipeStreamResult> {
        admin::wipe_stream(self, stream_key).await
    }

    async fn wipe_all(&self) -> Result<()> {
        admin::wipe_all(self).await
    }

    async fn delete_message(&self, stream_key: &str, sequence: u64) -> Result<DeleteMessageResult> {
        admin::delete_message(self, stream_key, sequence).await
    }

    async fn get_messages_batch(&self, params: BatchParams) -> Result<crate::types::BatchResult> {
        replay::get_messages_batch(self, params).await
    }

    async fn subscribe_to_topic(&self, topic: &str) -> Result<Subscription> {
        subscriber::subscribe_to_topic(self, topic).await
    }

    async fn history_end(&self, topic: &str) -> Result<u64> {
        let (pattern, _) = crate::notification::wildcard_matcher::analyze_watch_pattern(topic)?;
        let name = self
            .ensure_stream_for_topic(&pattern)
            .await
            .context("Failed to ensure stream for replay boundary")?;
        let stream = self
            .jetstream
            .get_stream(name)
            .await
            .context("Failed to capture replay boundary")?;
        Ok(stream.cached_info().state.last_sequence)
    }

    async fn shutdown(&self) -> Result<()> {
        connection::shutdown(self).await
    }
}
