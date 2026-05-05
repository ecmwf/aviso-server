// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use std::{net::TcpListener, sync::Arc};

use actix_web::{App, HttpServer, dev::Server, web};
use tokio::task;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};
use tracing_actix_web::TracingLogger;

use crate::auth::middleware::AuthMiddleware;
#[cfg(feature = "ecpds")]
use crate::configuration::validate_ecpds_settings;
use crate::configuration::{AuthSettings, validate_metrics_settings};
use crate::metrics::AppMetrics;
use crate::openapi::ApiDoc;
use crate::routes::admin::{delete_notification, wipe_all, wipe_stream};
use crate::routes::home::homepage;
use crate::routes::replay::replay;
use crate::routes::schema::{get_event_schema, get_notification_schema};
use crate::routes::watch::watch;
use crate::{
    configuration::{
        Settings, validate_auth_settings, validate_schema_storage_policy_support,
        validate_stream_auth_settings, validate_stream_plugin_settings,
    },
    notification_backend::{NotificationBackend, build_backend},
    routes::{health_check::health_check, notify::notify},
    telemetry::{SERVICE_NAME, SERVICE_VERSION},
};
use actix_files as fs;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

#[allow(dead_code)]
pub struct Application {
    port: u16,
    server: Server,
    metrics_server: Option<Server>,
    shutdown: CancellationToken,
    backend: Arc<dyn NotificationBackend>, // backend reference for shutdown
}

impl Application {
    // Build the server from the configuration
    pub async fn build(
        configuration: Settings,
        shutdown: CancellationToken,
    ) -> Result<Self, std::io::Error> {
        if let Err(e) = validate_schema_storage_policy_support(&configuration) {
            error!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_name = "startup.configuration.validation.failed",
                error = %e,
                "Configuration validation failed"
            );
            return Err(std::io::Error::other(e));
        }

        if let Err(e) = validate_auth_settings(&configuration.auth) {
            error!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_name = "startup.auth.validation.failed",
                error = %e,
                "Auth configuration validation failed"
            );
            return Err(std::io::Error::other(e));
        }

        if let Err(e) = validate_stream_plugin_settings(&configuration) {
            error!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_name = "startup.auth.plugin_validation.failed",
                error = %e,
                "Stream plugin configuration validation failed"
            );
            return Err(std::io::Error::other(e));
        }

        if let Err(e) = validate_stream_auth_settings(&configuration) {
            error!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_name = "startup.auth.stream_validation.failed",
                error = %e,
                "Stream auth configuration validation failed"
            );
            return Err(std::io::Error::other(e));
        }

        if let Err(e) = validate_metrics_settings(&configuration) {
            error!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_name = "startup.metrics.validation.failed",
                error = %e,
                "Metrics configuration validation failed"
            );
            return Err(std::io::Error::other(e));
        }

        #[cfg(feature = "ecpds")]
        let ecpds_checker: Option<Arc<aviso_ecpds::checker::EcpdsChecker>> = {
            if let Err(e) = validate_ecpds_settings(&configuration) {
                error!(
                    service_name = SERVICE_NAME,
                    service_version = SERVICE_VERSION,
                    event_name = "startup.ecpds.validation.failed",
                    error = %e,
                    "ECPDS configuration validation failed"
                );
                return Err(std::io::Error::other(e));
            }
            match configuration.build_ecpds_checker() {
                Ok(checker) => checker.map(Arc::new),
                Err(e) => {
                    error!(
                        service_name = SERVICE_NAME,
                        service_version = SERVICE_VERSION,
                        event_name = "startup.ecpds.checker_init.failed",
                        error = %e,
                        "ECPDS checker initialization failed"
                    );
                    return Err(std::io::Error::other(e));
                }
            }
        };

        let address = format!(
            "{}:{}",
            configuration.application.host, configuration.application.port
        );
        let listener = TcpListener::bind(&address)?;
        let port = listener.local_addr()?.port();

        // Initialize the configured notification backend before binding routes.
        let notification_backend = match build_backend(&configuration.notification_backend).await {
            Ok(backend) => backend,
            Err(e) => {
                error!(
                    service_name = SERVICE_NAME,
                    service_version = SERVICE_VERSION,
                    event_name = "startup.backend.initialization.failed",
                    error = %e,
                    "Failed to initialize notification backend"
                );
                return Err(std::io::Error::other(e));
            }
        };

        let (app_metrics, metrics_server) = if configuration.metrics.enabled {
            let metrics = AppMetrics::new();
            crate::metrics::register_process_metrics(&metrics.registry);

            let metrics_port = configuration.metrics.port.expect("validated above");
            let metrics_host = &configuration.metrics.host;
            let metrics_addr = format!("{metrics_host}:{metrics_port}");
            let metrics_listener = TcpListener::bind(&metrics_addr)?;

            info!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_name = "startup.metrics.server.binding",
                host = %metrics_host,
                port = metrics_port,
                "Metrics server binding"
            );

            let server =
                crate::metrics::run_metrics_server(metrics_listener, metrics.registry.clone())?;
            (Some(metrics), Some(server))
        } else {
            (None, None)
        };

        let server = run(
            listener,
            notification_backend.clone(),
            shutdown.clone(),
            Arc::new(configuration.auth.clone()),
            app_metrics,
            #[cfg(feature = "ecpds")]
            ecpds_checker,
        )?;

        // stop Actix when the cancellation token is triggered
        let handle = server.handle();
        let metrics_handle = metrics_server.as_ref().map(|s| s.handle());
        let backend_for_shutdown = notification_backend.clone();
        task::spawn({
            let token = shutdown.clone();
            async move {
                token.cancelled().await;

                info!(
                    service_name = SERVICE_NAME,
                    service_version = SERVICE_VERSION,
                    event_name = "startup.shutdown.received",
                    "Shutdown signal received, stopping Actix server"
                );

                // Stop metrics server gracefully, then the main server.
                if let Some(mh) = metrics_handle {
                    mh.stop(true).await;
                }
                handle.stop(true).await;

                info!(
                    service_name = SERVICE_NAME,
                    service_version = SERVICE_VERSION,
                    event_name = "startup.server.stopped",
                    "Actix server stopped, shutting down backend"
                );

                // Then shutdown the backend
                if let Err(e) = shutdown_backend(backend_for_shutdown).await {
                    error!(
                        service_name = SERVICE_NAME,
                        service_version = SERVICE_VERSION,
                        event_name = "startup.backend.shutdown.failed",
                        error = %e,
                        "Error during backend shutdown"
                    );
                } else {
                    info!(
                        service_name = SERVICE_NAME,
                        service_version = SERVICE_VERSION,
                        event_name = "startup.backend.shutdown.succeeded",
                        "Backend shutdown completed successfully"
                    );
                }
            }
        });

        Ok(Self {
            port,
            server,
            metrics_server,
            shutdown,
            backend: notification_backend,
        })
    }

    // This is to get the port number from the TcpListener
    // it is useful when a random port is used
    pub fn port(&self) -> u16 {
        self.port
    }

    // This function is used to run the server
    pub async fn run_until_stopped(self) -> Result<(), std::io::Error> {
        match self.metrics_server {
            Some(metrics) => {
                tokio::try_join!(self.server, metrics)?;
                Ok(())
            }
            None => self.server.await,
        }
    }
}

/// Shutdown the notification backend gracefully
///
/// This function calls the shutdown method on the NotificationBackend trait object,
/// allowing all backend implementations to handle their own cleanup.
async fn shutdown_backend(backend: Arc<dyn NotificationBackend>) -> anyhow::Result<()> {
    info!(
        service_name = SERVICE_NAME,
        service_version = SERVICE_VERSION,
        event_name = "startup.backend.shutdown.started",
        "Shutting down notification backend"
    );

    // Call the shutdown method defined in the trait
    backend.shutdown().await?;

    info!(
        service_name = SERVICE_NAME,
        service_version = SERVICE_VERSION,
        event_name = "startup.backend.shutdown.completed",
        "Notification backend shutdown completed"
    );
    Ok(())
}

/// Configure operational/infrastructure routes
fn configure_ops_routes(cfg: &mut web::ServiceConfig) {
    let static_path = Settings::get_global_application_settings()
        .static_files_path
        .clone();
    cfg.service(fs::Files::new("/static", static_path).show_files_listing())
        .route("/health", web::get().to(health_check))
        .route("/", web::get().to(homepage));
}

/// Configure API v1 routes
fn configure_api_v1(cfg: &mut web::ServiceConfig, auth_settings: Arc<AuthSettings>) {
    cfg.service(
        web::scope("/api/v1")
            .wrap(AuthMiddleware::with_arc_settings(auth_settings))
            .route("/notification", web::post().to(notify))
            .route("/watch", web::post().to(watch))
            .route("/replay", web::post().to(replay))
            .route("/schema", web::get().to(get_notification_schema))
            .route("/schema/{event_type}", web::get().to(get_event_schema))
            .service(
                web::scope("/admin")
                    .route("/wipe/stream", web::delete().to(wipe_stream))
                    .route("/wipe/all", web::delete().to(wipe_all))
                    .route(
                        "/notification/{notification_id}",
                        web::delete().to(delete_notification),
                    ),
            ),
    );
}

// Run the server
pub fn run(
    listener: TcpListener,
    notification_backend: Arc<dyn NotificationBackend>,
    shutdown: CancellationToken,
    auth_settings: Arc<AuthSettings>,
    app_metrics: Option<AppMetrics>,
    #[cfg(feature = "ecpds")] ecpds_checker: Option<Arc<aviso_ecpds::checker::EcpdsChecker>>,
) -> Result<Server, std::io::Error> {
    let metrics_data = app_metrics.map(web::Data::new);
    #[cfg(feature = "ecpds")]
    let ecpds_data = ecpds_checker.map(web::Data::new);
    let server = HttpServer::new(move || {
        let mut app = App::new()
            .wrap(TracingLogger::default())
            .service(
                SwaggerUi::new("/swagger-ui/{_:.*}")
                    .url("/api-docs/openapi.json", ApiDoc::openapi()),
            )
            .configure(configure_ops_routes)
            .configure({
                let auth_settings = Arc::clone(&auth_settings);
                move |cfg| configure_api_v1(cfg, Arc::clone(&auth_settings))
            })
            .app_data(web::Data::new(notification_backend.clone()))
            .app_data(web::Data::new(shutdown.clone()))
            .app_data(web::Data::new(auth_settings.clone()));

        if let Some(ref metrics) = metrics_data {
            app = app.app_data(metrics.clone());
        }

        #[cfg(feature = "ecpds")]
        if let Some(ref ecpds) = ecpds_data {
            app = app.app_data(ecpds.clone());
        }

        app
    })
    .listen(listener)?
    .shutdown_timeout(30)
    .disable_signals()
    .run();
    Ok(server)
}
