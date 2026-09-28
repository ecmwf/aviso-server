// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! A server with the notify, watch and replay routes over a given backend,
//! for tests that fill the backend directly and then read it over HTTP.

use actix_web::dev::ServerHandle;
use actix_web::{App, HttpServer, web};
use aviso_server::configuration::Settings;
use aviso_server::notification_backend::NotificationBackend;
use serde_json::Value;
use std::sync::Arc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub struct ReplayApp {
    /// Base URL; the routes are `/notification`, `/watch` and `/replay`.
    pub address: String,
    handle: ServerHandle,
    task: JoinHandle<std::io::Result<()>>,
}

impl ReplayApp {
    pub fn start(config: &Settings, backend: Arc<dyn NotificationBackend>) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let config = config.clone();
        let server = HttpServer::new(move || {
            App::new()
                .wrap(tracing_actix_web::TracingLogger::default())
                .app_data(web::Data::new(config.clone()))
                .app_data(web::Data::new(backend.clone()))
                .app_data(web::Data::new(CancellationToken::new()))
                .route(
                    "/notification",
                    web::post().to(aviso_server::routes::notify::notify),
                )
                .route("/watch", web::post().to(aviso_server::routes::watch::watch))
                .route(
                    "/replay",
                    web::post().to(aviso_server::routes::replay::replay),
                )
        })
        .workers(1)
        .listen(listener)
        .unwrap()
        .run();
        let handle = server.handle();
        let task = tokio::spawn(server);
        Self {
            address,
            handle,
            task,
        }
    }

    pub async fn stop(self) {
        self.handle.stop(true).await;
        self.task.await.unwrap().unwrap();
    }
}

/// The JSON `data:` payloads of a finished SSE stream, in order.
pub fn sse_data_events(body: &str) -> Vec<Value> {
    body.lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(|data| serde_json::from_str(data.trim()).unwrap())
        .collect()
}
