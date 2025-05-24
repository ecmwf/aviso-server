use aviso_server::{
    configuration::get_configuration,
    startup::Application,
    telemetry::{get_subscriber, init_subscriber},
};
use tracing::error;

#[tokio::main]
async fn main() -> Result<(), std::io::Error> {
    let configuration = match get_configuration() {
        Ok(cfg) => cfg,
        Err(e) => {
            error!("Failed to load configuration: {e}");
            return Err(std::io::Error::new(std::io::ErrorKind::Other, e));
        }
    };

    let subscriber = get_subscriber(
        "aviso-server".into(),
        configuration.logging.as_ref(),
        std::io::stdout,
    );
    init_subscriber(subscriber);

    let application = match Application::build(configuration).await {
        Ok(app) => app,
        Err(e) => {
            error!("Failed to build application: {e}");
            return Err(e);
        }
    };

    if let Err(e) = application.run_until_stopped().await {
        error!("Application error: {e}");
        return Err(e);
    }
    Ok(())
}
