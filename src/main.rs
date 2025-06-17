use aviso_server::{
    configuration::get_configuration,
    startup::Application,
    telemetry::{get_subscriber, init_subscriber},
};
use tokio::signal;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

#[tokio::main]
async fn main() -> Result<(), std::io::Error> {
    let configuration = match get_configuration() {
        Ok(cfg) => cfg,
        Err(e) => {
            error!("Failed to load configuration: {e}");
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

    // Log a simple message first
    info!("Starting server with configuration:");

    // Then output the raw JSON configuration directly to stdout
    match serde_json::to_string_pretty(&configuration) {
        Ok(config_json) => println!("{config_json}"),
        Err(e) => warn!(error = %e, "Failed to serialize configuration to JSON"),
    }

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
                    info!("Received SIGINT (Ctrl+C), initiating graceful shutdown");
                },
                _ = term_stream.recv() => {
                    info!("Received SIGTERM, initiating graceful shutdown");
                },
            }
        }

        #[cfg(not(unix))]
        {
            let _ = signal::ctrl_c().await;
            info!("Received Ctrl+C, initiating graceful shutdown");
        }

        info!("Shutdown signal received – cancelling token");
        shutdown_signal.cancel();
    });

    // pass the token into the application builder
    let application = Application::build(configuration, shutdown).await?;
    application.run_until_stopped().await
}
