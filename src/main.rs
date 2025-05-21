use aviso_server::configuration::get_configuration;
use aviso_server::startup::Application;
use aviso_server::telemetry::{get_subscriber, init_subscriber};

#[tokio::main]
async fn main() -> Result<(), std::io::Error> {
    // set tracing subscriber
    let subscriber = get_subscriber("zero2prod".into(), "info".into(), std::io::stdout);
    init_subscriber(subscriber);
    // Load the configuration
    let configuration = get_configuration().expect("Failed to load configuration");
    // Build the server
    let application = Application::build(configuration).await?;
    // Run the server until stopped
    application.run_until_stopped().await?;
    Ok(())
}
