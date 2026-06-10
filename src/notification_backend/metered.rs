// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! `NotificationBackend` decorator that records per-operation Prometheus
//! metrics (`aviso_backend_operations_total`,
//! `aviso_backend_operation_duration_seconds`) at the trait boundary.
//!
//! Timing happens around the inner call, so it measures caller-observed
//! latency for every backend implementation without threading metrics into
//! each one. `subscribe_to_topic` is intentionally NOT timed: it only sets up
//! the subscription, and the actual message flow happens later as the returned
//! stream is polled, so a duration there would be misleading.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use async_trait::async_trait;
use futures::Stream;

use super::replay::BatchParams;
use super::{BackendCapabilities, DeleteMessageResult, NotificationBackend, NotificationMessage};
use crate::metrics::AppMetrics;
use crate::types::BatchResult;

/// Backend operations recorded by the metered decorator. Static strings keep
/// the `operation` label a fixed, low-cardinality enum.
const OP_PUBLISH: &str = "publish";
const OP_GET_BATCH: &str = "get_batch";
const OP_WIPE_STREAM: &str = "wipe_stream";
const OP_WIPE_ALL: &str = "wipe_all";
const OP_DELETE_MESSAGE: &str = "delete_message";

const ALL_OPERATIONS: [&str; 5] = [
    OP_PUBLISH,
    OP_GET_BATCH,
    OP_WIPE_STREAM,
    OP_WIPE_ALL,
    OP_DELETE_MESSAGE,
];

pub struct MeteredBackend {
    inner: Arc<dyn NotificationBackend>,
    metrics: AppMetrics,
    backend: String,
}

impl MeteredBackend {
    pub fn new(inner: Arc<dyn NotificationBackend>, metrics: AppMetrics, backend: &str) -> Self {
        // Pre-initialise the bounded {backend, operation, outcome} series at
        // zero so rate()/alert rules evaluate against existing series from
        // startup (same rationale as the other pre-inits in metrics.rs).
        for operation in ALL_OPERATIONS {
            for outcome in ["ok", "error"] {
                let _ = metrics
                    .backend_operations_total
                    .with_label_values(&[backend, operation, outcome]);
                let _ = metrics
                    .backend_operation_duration_seconds
                    .with_label_values(&[backend, operation, outcome]);
            }
        }
        Self {
            inner,
            metrics,
            backend: backend.to_string(),
        }
    }

    fn record(&self, operation: &str, started_at: Instant, is_ok: bool) {
        let outcome = if is_ok { "ok" } else { "error" };
        self.metrics
            .backend_operations_total
            .with_label_values(&[&self.backend, operation, outcome])
            .inc();
        self.metrics
            .backend_operation_duration_seconds
            .with_label_values(&[&self.backend, operation, outcome])
            .observe(started_at.elapsed().as_secs_f64());
    }
}

#[async_trait]
impl NotificationBackend for MeteredBackend {
    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }

    async fn put_messages(&self, topic: &str, payload: String) -> Result<()> {
        let started = Instant::now();
        let result = self.inner.put_messages(topic, payload).await;
        self.record(OP_PUBLISH, started, result.is_ok());
        result
    }

    async fn put_message_with_headers(
        &self,
        topic: &str,
        headers: Option<HashMap<String, String>>,
        payload: String,
    ) -> Result<()> {
        let started = Instant::now();
        let result = self
            .inner
            .put_message_with_headers(topic, headers, payload)
            .await;
        self.record(OP_PUBLISH, started, result.is_ok());
        result
    }

    async fn wipe_stream(&self, stream_name: &str) -> Result<()> {
        let started = Instant::now();
        let result = self.inner.wipe_stream(stream_name).await;
        self.record(OP_WIPE_STREAM, started, result.is_ok());
        result
    }

    async fn wipe_all(&self) -> Result<()> {
        let started = Instant::now();
        let result = self.inner.wipe_all().await;
        self.record(OP_WIPE_ALL, started, result.is_ok());
        result
    }

    async fn delete_message(&self, stream_key: &str, sequence: u64) -> Result<DeleteMessageResult> {
        let started = Instant::now();
        let result = self.inner.delete_message(stream_key, sequence).await;
        self.record(OP_DELETE_MESSAGE, started, result.is_ok());
        result
    }

    async fn get_messages_batch(&self, params: BatchParams) -> Result<BatchResult> {
        let started = Instant::now();
        let result = self.inner.get_messages_batch(params).await;
        self.record(OP_GET_BATCH, started, result.is_ok());
        result
    }

    async fn subscribe_to_topic(
        &self,
        topic: &str,
    ) -> Result<Box<dyn Stream<Item = NotificationMessage> + Unpin + Send>> {
        // Not timed: see module docs (lazy stream, setup duration is misleading).
        self.inner.subscribe_to_topic(topic).await
    }

    async fn shutdown(&self) -> Result<()> {
        self.inner.shutdown().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::BatchResult;
    use async_trait::async_trait;

    struct FakeBackend {
        fail: bool,
    }

    #[async_trait]
    impl NotificationBackend for FakeBackend {
        fn capabilities(&self) -> BackendCapabilities {
            super::super::IN_MEMORY_CAPABILITIES
        }
        async fn put_messages(&self, _topic: &str, _payload: String) -> Result<()> {
            if self.fail {
                anyhow::bail!("boom");
            }
            Ok(())
        }
        async fn put_message_with_headers(
            &self,
            _topic: &str,
            _headers: Option<HashMap<String, String>>,
            _payload: String,
        ) -> Result<()> {
            Ok(())
        }
        async fn wipe_stream(&self, _stream_name: &str) -> Result<()> {
            Ok(())
        }
        async fn wipe_all(&self) -> Result<()> {
            Ok(())
        }
        async fn delete_message(
            &self,
            _stream_key: &str,
            _sequence: u64,
        ) -> Result<DeleteMessageResult> {
            Ok(DeleteMessageResult::NotFound)
        }
        async fn get_messages_batch(&self, _params: BatchParams) -> Result<BatchResult> {
            Ok(BatchResult::new(Vec::new(), 0))
        }
        async fn subscribe_to_topic(
            &self,
            _topic: &str,
        ) -> Result<Box<dyn Stream<Item = NotificationMessage> + Unpin + Send>> {
            Ok(Box::new(futures::stream::empty()))
        }
    }

    fn op_count(m: &AppMetrics, op: &str, outcome: &str) -> u64 {
        m.backend_operations_total
            .with_label_values(&["in_memory", op, outcome])
            .get()
    }

    fn op_samples(m: &AppMetrics, op: &str, outcome: &str) -> u64 {
        m.backend_operation_duration_seconds
            .with_label_values(&["in_memory", op, outcome])
            .get_sample_count()
    }

    #[tokio::test]
    async fn records_ok_publish() {
        let metrics = AppMetrics::new();
        let backend = MeteredBackend::new(
            Arc::new(FakeBackend { fail: false }),
            metrics.clone(),
            "in_memory",
        );

        backend
            .put_messages("topic", "payload".into())
            .await
            .unwrap();

        assert_eq!(op_count(&metrics, "publish", "ok"), 1);
        assert_eq!(op_samples(&metrics, "publish", "ok"), 1);
        assert_eq!(op_count(&metrics, "publish", "error"), 0);
    }

    #[tokio::test]
    async fn records_error_outcome_and_propagates() {
        let metrics = AppMetrics::new();
        let backend = MeteredBackend::new(
            Arc::new(FakeBackend { fail: true }),
            metrics.clone(),
            "in_memory",
        );

        let result = backend.put_messages("topic", "payload".into()).await;
        assert!(result.is_err(), "error must propagate unchanged");

        assert_eq!(op_count(&metrics, "publish", "error"), 1);
        assert_eq!(op_count(&metrics, "publish", "ok"), 0);
    }

    #[tokio::test]
    async fn subscribe_is_not_timed() {
        let metrics = AppMetrics::new();
        let backend = MeteredBackend::new(
            Arc::new(FakeBackend { fail: false }),
            metrics.clone(),
            "in_memory",
        );

        let _stream = backend.subscribe_to_topic("topic").await.unwrap();

        // No "subscribe" operation series should ever be created.
        let encoder = prometheus::TextEncoder::new();
        let mut buf = Vec::new();
        prometheus::Encoder::encode(&encoder, &metrics.registry.gather(), &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert!(
            !output.contains(r#"operation="subscribe""#),
            "subscribe must not be recorded as a backend operation"
        );
    }

    #[tokio::test]
    async fn series_preinitialized_at_zero() {
        let metrics = AppMetrics::new();
        let _backend = MeteredBackend::new(
            Arc::new(FakeBackend { fail: false }),
            metrics.clone(),
            "in_memory",
        );

        // Read the scrape text WITHOUT touching with_label_values (which would
        // create-on-read and mask a missing pre-init): the series must already
        // be present at zero purely from MeteredBackend::new.
        let encoder = prometheus::TextEncoder::new();
        let mut buf = Vec::new();
        prometheus::Encoder::encode(&encoder, &metrics.registry.gather(), &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();

        for op in [
            "publish",
            "get_batch",
            "wipe_stream",
            "wipe_all",
            "delete_message",
        ] {
            for outcome in ["ok", "error"] {
                let counter = format!(
                    r#"aviso_backend_operations_total{{backend="in_memory",operation="{op}",outcome="{outcome}"}} 0"#
                );
                assert!(
                    output.contains(&counter),
                    "counter series should be pre-initialised at zero: {counter}"
                );
                let histogram = format!(
                    r#"aviso_backend_operation_duration_seconds_count{{backend="in_memory",operation="{op}",outcome="{outcome}"}} 0"#
                );
                assert!(
                    output.contains(&histogram),
                    "histogram series should be pre-initialised at zero: {histogram}"
                );
            }
        }
    }
}
