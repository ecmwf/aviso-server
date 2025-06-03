use std::{net::TcpListener, sync::Arc};

use actix_web::{App, HttpServer, dev::Server, web};
use tracing_actix_web::TracingLogger;

use crate::routes::schema::{get_event_schema, get_notification_schema};
use crate::{
    configuration::Settings,
    notification_backend::{NotificationBackend, build_backend},
    routes::{health_check::health_check, notify::notify},
};

pub struct Application {
    port: u16,
    server: Server,
}

impl Application {
    // Build the server from the configuration
    pub async fn build(configuration: Settings) -> Result<Self, std::io::Error> {
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
                tracing::error!("Failed to initialize notification backend: {e}");
                return Err(std::io::Error::new(std::io::ErrorKind::Other, e));
            }
        };
        let server = run(listener, notification_backend)?;
        Ok(Self { port, server })
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

/// Configure operational/infrastructure routes
fn configure_ops_routes(cfg: &mut web::ServiceConfig) {
    cfg.route("/health", web::get().to(health_check));
}

/// Configure API v1 routes
fn configure_api_v1(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/api/v1")
            .route("/notification", web::post().to(notify))
            .route("/schema", web::get().to(get_notification_schema))
            .route("/schema/{event_type}", web::get().to(get_event_schema)),
    );
}

// Run the server
pub fn run(
    listener: TcpListener,
    notification_backend: Arc<dyn NotificationBackend>,
) -> Result<Server, std::io::Error> {
    let server = HttpServer::new(move || {
        App::new()
            .wrap(TracingLogger::default())
            .configure(configure_ops_routes)
            .configure(configure_api_v1)
            .app_data(web::Data::new(notification_backend.clone()))
    })
    .listen(listener)?
    .run();
    Ok(server)
}
