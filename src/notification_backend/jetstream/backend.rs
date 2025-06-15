use crate::notification_backend::jetstream::{admin, connection, publisher, streams, subscriber};
use crate::notification_backend::{NotificationBackend, NotificationMessage};
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures_util::Stream;

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
    async fn put_messages(&self, topic: &str, payload: String) -> Result<()> {
        publisher::put_messages(self, topic, payload).await
    }

    async fn wipe_stream(&self, stream_name: &str) -> Result<()> {
        admin::wipe_stream(self, stream_name).await
    }

    async fn wipe_all(&self) -> Result<()> {
        admin::wipe_all(self).await
    }

    async fn get_messages_batch(
        &self,
        topic: &str,
        from_sequence: Option<u64>,
        from_date: Option<DateTime<Utc>>,
        limit: usize,
        offset: usize,
    ) -> Result<(Vec<NotificationMessage>, bool)> {
        subscriber::get_messages_batch(self, topic, from_sequence, from_date, limit, offset).await
    }

    async fn count_messages(
        &self,
        topic: &str,
        from_sequence: Option<u64>,
        from_date: Option<DateTime<Utc>>,
    ) -> Result<usize> {
        subscriber::count_messages(self, topic, from_sequence, from_date).await
    }

    async fn subscribe_to_topic(
        &self,
        topic: &str,
    ) -> Result<Box<dyn Stream<Item = NotificationMessage> + Unpin + Send>> {
        subscriber::subscribe_to_topic(self, topic).await
    }

    async fn shutdown(&self) -> Result<()> {
        connection::shutdown(self).await
    }
}
