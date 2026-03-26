// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use super::Settings;
use crate::telemetry::{SERVICE_NAME, SERVICE_VERSION};

const CONFIG_FILE_ENV: &str = "AVISOSERVER_CONFIG_FILE";

/// Loads configuration with explicit precedence (last source wins):
///
/// When `AVISOSERVER_CONFIG_FILE` is set, that file is used as the only file
/// source (startup fails if the file does not exist). Otherwise the default
/// cascade applies:
/// 1) `./configuration/config.yaml`
/// 2) `/etc/aviso_server/config.yaml`
/// 3) `$HOME/.aviso_server/config.yaml`
///
/// In both cases `AVISOSERVER_*` environment variables are applied last.
///
/// Example env override:
/// `AVISOSERVER_NOTIFICATION_BACKEND__KIND=jetstream`
pub fn get_configuration() -> Result<Settings, config::ConfigError> {
    let mut settings = config::Config::builder();

    settings = match std::env::var(CONFIG_FILE_ENV) {
        Err(std::env::VarError::NotPresent) => add_default_file_sources(settings),
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(config::ConfigError::NotFound(format!(
                "{CONFIG_FILE_ENV} contains non-Unicode value"
            )));
        }
        Ok(raw) => {
            let config_file = raw.trim().to_string();
            if config_file.is_empty() {
                return Err(config::ConfigError::NotFound(format!(
                    "{CONFIG_FILE_ENV} is set but empty"
                )));
            }
            let path = std::path::Path::new(&config_file);
            if !path.is_file() {
                return Err(config::ConfigError::NotFound(format!(
                    "{CONFIG_FILE_ENV} does not point to a file: {config_file}"
                )));
            }
            tracing::info!(path = %config_file, env = CONFIG_FILE_ENV, "Loading configuration from override env var");
            settings.add_source(config::File::from(path.to_path_buf()))
        }
    };

    settings = settings.add_source(
        config::Environment::with_prefix("AVISOSERVER")
            .prefix_separator("_")
            .separator("__"),
    );

    let settings = settings.build()?.try_deserialize::<Settings>()?;

    tracing::info!(
        service_name = SERVICE_NAME,
        service_version = SERVICE_VERSION,
        event_name = "configuration.load.succeeded",
        host = %settings.application.host,
        port = settings.application.port,
        backend_kind = %settings.notification_backend.kind,
        has_notification_schema = settings.notification_schema.is_some(),
        "Configuration loaded successfully"
    );

    Ok(settings)
}

fn add_default_file_sources(
    mut settings: config::ConfigBuilder<config::builder::DefaultState>,
) -> config::ConfigBuilder<config::builder::DefaultState> {
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

    settings
}
