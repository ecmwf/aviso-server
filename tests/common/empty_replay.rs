// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use anyhow::Result;
use aviso_server::notification_backend::{
    BackendCapabilities, DeleteMessageResult, IN_MEMORY_CAPABILITIES, NotificationBackend,
    Subscription, WipeStreamResult, replay::BatchParams,
};
use aviso_server::types::BatchResult;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio_util::sync::CancellationToken;

pub struct EmptyReplay {
    pub calls: AtomicUsize,
    pub shutdown: CancellationToken,
}

#[async_trait::async_trait]
impl NotificationBackend for EmptyReplay {
    fn capabilities(&self) -> BackendCapabilities {
        IN_MEMORY_CAPABILITIES
    }

    async fn get_messages_batch(&self, _params: BatchParams) -> Result<BatchResult> {
        let calls = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if calls == 2 {
            self.shutdown.cancel();
        }
        // Model backend pattern filtering while new history keeps arriving.
        // The finite fallback makes a missing cooperative yield fail, not hang.
        let mut batch = BatchResult::empty();
        batch.has_more = calls < 1000;
        batch.next_sequence = Some(u64::try_from(calls).unwrap() + 1);
        Ok(batch)
    }

    async fn put_messages(&self, _: &str, _: String) -> Result<()> {
        panic!("replay must not publish")
    }
    async fn put_message_with_headers(
        &self,
        _: &str,
        _: Option<HashMap<String, String>>,
        _: String,
    ) -> Result<()> {
        panic!("replay must not publish")
    }
    async fn wipe_stream(&self, _: &str) -> Result<WipeStreamResult> {
        panic!("replay must not wipe storage")
    }
    async fn wipe_all(&self) -> Result<()> {
        panic!("replay must not wipe storage")
    }
    async fn delete_message(&self, _: &str, _: u64) -> Result<DeleteMessageResult> {
        panic!("replay must not delete")
    }
    async fn subscribe_to_topic(&self, _: &str) -> Result<Subscription> {
        panic!("replay-only must not subscribe")
    }
    async fn history_end(&self, _: &str) -> Result<u64> {
        Ok(1000)
    }
}
