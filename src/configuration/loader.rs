use super::Settings;
use crate::telemetry::{SERVICE_NAME, SERVICE_VERSION};

/// Loads configuration with explicit precedence (last source wins):
/// 1) `./configuration/config.yaml`
/// 2) `/etc/aviso_server/config.yaml`
/// 3) `$HOME/.aviso_server/config.yaml`
/// 4) `AVISOSERVER_*` environment variables.
///
/// If the same field appears in multiple sources, the later source overrides it.
///
/// Example env override:
/// `AVISOSERVER_NOTIFICATION_BACKEND__KIND=jetstream`
pub fn get_configuration() -> Result<Settings, config::ConfigError> {
    let mut settings = config::Config::builder();

    let base_path = std::env::current_dir().expect("Failed to get current directory");
    let config_dir = base_path.join("configuration").join("config.yaml");
    if config_dir.exists() {
        tracing::debug!(path = ?config_dir, "Loading base configuration");
        settings = settings.add_source(config::File::from(config_dir));
    }

    let etc_path = "/etc/aviso_server/config.yaml";
    if std::path::Path::new(etc_path).exists() {
        tracing::debug!(path = etc_path, "Loading system configuration");
        settings = settings.add_source(config::File::with_name(etc_path).required(false));
    }

    if let Some(home_dir) = dirs::home_dir() {
        let user_config_path = home_dir.join(".aviso_server/config.yaml");
        if user_config_path.exists() {
            tracing::debug!(path = ?user_config_path, "Loading user configuration");
            settings = settings.add_source(config::File::from(user_config_path).required(false));
        }
    }

    settings = settings.add_source(
        config::Environment::with_prefix("AVISOSERVER")
            .prefix_separator("_")
            .separator("__"),
    );

    let settings = settings.build()?.try_deserialize::<Settings>()?;

    tracing::info!(
        service_name = SERVICE_NAME,
        service_version = SERVICE_VERSION,
        event_domain = "configuration",
        event_name = "configuration.load.succeeded",
        host = %settings.application.host,
        port = settings.application.port,
        backend_kind = %settings.notification_backend.kind,
        has_notification_schema = settings.notification_schema.is_some(),
        "Configuration loaded successfully"
    );

    Ok(settings)
}
