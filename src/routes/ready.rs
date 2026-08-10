// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use crate::notification_backend::NotificationBackend;
use actix_web::{HttpResponse, web};
use serde::Serialize;
use std::sync::Arc;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct ReadyResponse {
    #[schema(example = "ready")]
    status: &'static str,
    /// Whether the notification backend's connection is currently healthy.
    #[schema(example = true)]
    backend_connected: bool,
}

/// Readiness probe: ready only when the notification backend connection is
/// healthy.
///
/// Complements `/health`, which stays a pure process-liveness signal: a
/// backend outage must not kill the pod (the client reconnects on its own),
/// but the pod should stop receiving traffic until the backend is back.
#[utoipa::path(
    get,
    path = "/ready",
    tag = "health",
    responses(
        (status = 200, description = "Service is ready to serve traffic", body = ReadyResponse),
        (status = 503, description = "Notification backend connection is down", body = ReadyResponse)
    )
)]
pub async fn ready(backend: web::Data<Arc<dyn NotificationBackend>>) -> HttpResponse {
    if backend.connection_healthy() {
        HttpResponse::Ok().json(ReadyResponse {
            status: "ready",
            backend_connected: true,
        })
    } else {
        HttpResponse::ServiceUnavailable().json(ReadyResponse {
            status: "not_ready",
            backend_connected: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notification_backend::{
        BackendCapabilities, DeleteMessageResult, NotificationMessage,
    };
    use async_trait::async_trait;
    use futures::Stream;
    use std::collections::HashMap;

    /// Backend stub with a scripted connection state. Only
    /// `connection_healthy` and `capabilities` are reachable from the
    /// handler under test; the data-path methods are unreachable and
    /// panic if a future refactor starts calling them from `/ready`.
    struct ScriptedBackend {
        healthy: bool,
    }

    #[async_trait]
    impl NotificationBackend for ScriptedBackend {
        fn capabilities(&self) -> BackendCapabilities {
            BackendCapabilities {
                retention_time: false,
                max_messages: false,
                max_size: false,
                allow_duplicates: false,
                compression: false,
            }
        }

        fn connection_healthy(&self) -> bool {
            self.healthy
        }

        async fn put_messages(&self, _topic: &str, _payload: String) -> anyhow::Result<()> {
            unreachable!("readiness must not touch the data path")
        }

        async fn put_message_with_headers(
            &self,
            _topic: &str,
            _headers: Option<HashMap<String, String>>,
            _payload: String,
        ) -> anyhow::Result<()> {
            unreachable!("readiness must not touch the data path")
        }

        async fn wipe_stream(&self, _stream_name: &str) -> anyhow::Result<()> {
            unreachable!("readiness must not touch the data path")
        }

        async fn wipe_all(&self) -> anyhow::Result<()> {
            unreachable!("readiness must not touch the data path")
        }

        async fn delete_message(
            &self,
            _stream_key: &str,
            _sequence: u64,
        ) -> anyhow::Result<DeleteMessageResult> {
            unreachable!("readiness must not touch the data path")
        }

        async fn get_messages_batch(
            &self,
            _params: crate::notification_backend::replay::BatchParams,
        ) -> anyhow::Result<crate::types::BatchResult> {
            unreachable!("readiness must not touch the data path")
        }

        async fn subscribe_to_topic(
            &self,
            _topic: &str,
        ) -> anyhow::Result<Box<dyn Stream<Item = NotificationMessage> + Unpin + Send>> {
            unreachable!("readiness must not touch the data path")
        }
    }

    async fn ready_status(healthy: bool) -> actix_web::http::StatusCode {
        let backend: Arc<dyn NotificationBackend> = Arc::new(ScriptedBackend { healthy });
        let response = ready(web::Data::new(backend)).await;
        response.status()
    }

    #[tokio::test]
    async fn ready_returns_200_when_backend_connection_is_healthy() {
        assert_eq!(ready_status(true).await, actix_web::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn ready_returns_503_when_backend_connection_is_down() {
        assert_eq!(
            ready_status(false).await,
            actix_web::http::StatusCode::SERVICE_UNAVAILABLE
        );
    }
}
