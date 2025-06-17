use std::{net::TcpListener, sync::Arc};

use actix_web::{App, HttpServer, dev::Server, web};
use tokio::task;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};
use tracing_actix_web::TracingLogger;

use crate::routes::admin::{wipe_all, wipe_stream};
use crate::routes::schema::{get_event_schema, get_notification_schema};
use crate::routes::watch::watch;
use crate::{
    configuration::Settings,
    notification_backend::{NotificationBackend, build_backend},
    routes::{health_check::health_check, notify::notify},
};

#[allow(dead_code)]
pub struct Application {
    port: u16,
    server: Server,
    shutdown: CancellationToken,
    backend: Arc<dyn NotificationBackend>, // backend reference for shutdown
}

impl Application {
    // Build the server from the configuration
    pub async fn build(
        configuration: Settings,
        shutdown: CancellationToken,
    ) -> Result<Self, std::io::Error> {
        let address = format!(
            "{}:{}",
            configuration.application.host, configuration.application.port
        );
        let listener = TcpListener::bind(&address)?;
        let port = listener.local_addr()?.port();

        // notification notification_backend
        let notification_backend = match build_backend(&configuration.notification_backend).await {
            Ok(backend) => backend,
            Err(e) => {
                error!("Failed to initialize notification backend: {e}");
                return Err(std::io::Error::other(e));
            }
        };

        let server = run(listener, notification_backend.clone(), shutdown.clone())?;

        // stop Actix when the cancellation token is triggered
        let handle = server.handle();
        let backend_for_shutdown = notification_backend.clone();
        task::spawn({
            let token = shutdown.clone();
            async move {
                token.cancelled().await;

                info!("Shutdown signal received, stopping Actix server");

                // First, stop accepting new connections
                handle.stop(true).await;

                info!("Actix server stopped, shutting down backend");

                // Then shutdown the backend
                if let Err(e) = shutdown_backend(backend_for_shutdown).await {
                    error!("Error during backend shutdown: {}", e);
                } else {
                    info!("Backend shutdown completed successfully");
                }
            }
        });

        Ok(Self {
            port,
            server,
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
        self.server.await
    }
}

/// Shutdown the notification backend gracefully
///
/// This function calls the shutdown method on the NotificationBackend trait object,
/// allowing all backend implementations to handle their own cleanup.
async fn shutdown_backend(backend: Arc<dyn NotificationBackend>) -> anyhow::Result<()> {
    info!("Shutting down notification backend");

    // Call the shutdown method defined in the trait
    backend.shutdown().await?;

    info!("Notification backend shutdown completed");
    Ok(())
}

/// Configure operational/infrastructure routes
fn configure_ops_routes(cfg: &mut web::ServiceConfig) {
    cfg.route("/health", web::get().to(health_check));
}

/// Configure API v1 routes
fn configure_api_v1(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/api/v1")
            .route("/notification", web::post().to(notify))
            .route("/watch", web::post().to(watch))
            .route("/schema", web::get().to(get_notification_schema))
            .route("/schema/{event_type}", web::get().to(get_event_schema))
            .service(
                web::scope("/admin")
                    .route("/wipe/stream", web::delete().to(wipe_stream))
                    .route("/wipe/all", web::delete().to(wipe_all)),
            ),
    );
}

// Run the server
pub fn run(
    listener: TcpListener,
    notification_backend: Arc<dyn NotificationBackend>,
    shutdown: CancellationToken,
) -> Result<Server, std::io::Error> {
    let server = HttpServer::new(move || {
        App::new()
            .wrap(TracingLogger::default())
            .configure(configure_ops_routes)
            .configure(configure_api_v1)
            .app_data(web::Data::new(notification_backend.clone()))
            .app_data(web::Data::new(shutdown.clone()))
    })
    .listen(listener)?
    .shutdown_timeout(30)
    .disable_signals()
    .run();
    Ok(server)
}
