use crate::notification_backend::jetstream::{
    admin, connection, publisher, replay, streams, subscriber,
};
use crate::notification_backend::replay::BatchParams;
use crate::notification_backend::{NotificationBackend, NotificationMessage};
use anyhow::Result;
use async_trait::async_trait;
use futures_util::Stream;
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

    async fn wipe_stream(&self, stream_name: &str) -> Result<()> {
        admin::wipe_stream(self, stream_name).await
    }

    async fn wipe_all(&self) -> Result<()> {
        admin::wipe_all(self).await
    }

    async fn get_messages_batch(&self, params: BatchParams) -> Result<crate::types::BatchResult> {
        replay::get_messages_batch(self, params).await
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
