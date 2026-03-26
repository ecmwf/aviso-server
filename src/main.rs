// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use aviso_server::{
    configuration::Settings,
    configuration::get_configuration,
    startup::Application,
    telemetry::{
        SERVICE_NAME, SERVICE_VERSION, get_subscriber, init_subscriber, is_sensitive_key,
        redact_url_userinfo,
    },
};
use serde_json::{Map, Value, json};
use tokio::signal;
use tokio_util::sync::CancellationToken;
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), std::io::Error> {
    let configuration = match get_configuration() {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!(
                "Failed to load configuration (service_name={}, service_version={}): {}",
                SERVICE_NAME, SERVICE_VERSION, e
            );
            return Err(std::io::Error::other(e));
        }
    };

    // Initialize global configuration once
    configuration.init_global_config();

    let subscriber = get_subscriber(
        "aviso-server".into(),
        configuration.logging.as_ref(),
        std::io::stdout,
    );
    init_subscriber(subscriber);

    let redacted_config = redacted_config_json(&configuration);
    info!(
        service_name = SERVICE_NAME,
        service_version = SERVICE_VERSION,
        event_name = "startup.configuration.dumped",
        config = %redacted_config,
        "Server effective configuration (redacted)"
    );

    // Log startup configuration summary without dumping raw config values.
    info!(
        service_name = SERVICE_NAME,
        service_version = SERVICE_VERSION,
        event_name = "startup.configuration.loaded",
        bind_host = %configuration.application.host,
        bind_port = configuration.application.port,
        backend_kind = %configuration.notification_backend.kind,
        has_notification_schema = configuration.notification_schema.is_some(),
        "Server configuration loaded"
    );

    // create a global cancellation token that all components can observe
    let shutdown = CancellationToken::new();
    let shutdown_signal = shutdown.clone();

    tokio::spawn(async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};

            let mut term_stream =
                signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");

            tokio::select! {
                _ = signal::ctrl_c() => {
                    info!(
                        service_name = SERVICE_NAME,
                        service_version = SERVICE_VERSION,
                        event_name = "startup.signal.sigint.received",
                        "Received SIGINT (Ctrl+C), initiating graceful shutdown"
                    );
                },
                _ = term_stream.recv() => {
                    info!(
                        service_name = SERVICE_NAME,
                        service_version = SERVICE_VERSION,
                        event_name = "startup.signal.sigterm.received",
                        "Received SIGTERM, initiating graceful shutdown"
                    );
                },
            }
        }

        #[cfg(not(unix))]
        {
            let _ = signal::ctrl_c().await;
            info!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_name = "startup.signal.ctrlc.received",
                "Received Ctrl+C, initiating graceful shutdown"
            );
        }

        info!(
            service_name = SERVICE_NAME,
            service_version = SERVICE_VERSION,
            event_name = "startup.shutdown.token.cancelled",
            "Shutdown signal received – cancelling token"
        );
        shutdown_signal.cancel();
    });

    let host = configuration.application.host.clone();
    // pass the token into the application builder
    let application = Application::build(configuration, shutdown).await?;
    info!(
        service_name = SERVICE_NAME,
        service_version = SERVICE_VERSION,
        event_name = "startup.server.started",
        port = application.port(),
        swagger_url = format!("{}:{}/swagger-ui/", host, application.port()),
        "Server starting with OpenAPI documentation"
    );
    application.run_until_stopped().await
}

fn redacted_config_json(configuration: &Settings) -> Value {
    let value = serde_json::to_value(configuration).unwrap_or_else(|_| json!({}));
    redact_value_recursive(value)
}

fn redact_value_recursive(value: Value) -> Value {
    match value {
        Value::Object(obj) => {
            let mut redacted = Map::new();
            for (key, child) in obj {
                if is_sensitive_key(&key) {
                    redacted.insert(key, json!("[REDACTED]"));
                } else {
                    redacted.insert(key, redact_value_recursive(child));
                }
            }
            Value::Object(redacted)
        }
        Value::Array(items) => {
            Value::Array(items.into_iter().map(redact_value_recursive).collect())
        }
        Value::String(s) => {
            if let Some(redacted_url) = redact_url_userinfo(&s) {
                Value::String(redacted_url)
            } else {
                Value::String(s)
            }
        }
        other => other,
    }
}
