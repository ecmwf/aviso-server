use crate::configuration::Settings;
use crate::routes::health_check::health_check;
use actix_web::dev::Server;
use actix_web::{App, HttpServer, web};
use std::net::TcpListener;
use tracing_actix_web::TracingLogger;

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

        let server = run(listener)?;
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

// Run the server
pub fn run(listener: TcpListener) -> Result<Server, std::io::Error> {
    let server = HttpServer::new(move || {
        App::new()
            .wrap(TracingLogger::default())
            .route("/health", web::get().to(health_check))
    })
    .listen(listener)?
    .run();

    Ok(server)
}
