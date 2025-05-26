use dirs;
use serde::Deserialize;
use serde_aux::field_attributes::deserialize_number_from_string;
use std::collections::HashMap;

// NOTIFICATION SETTINGS
#[derive(Deserialize, Clone, Debug)]
pub struct PayloadConfig {
    pub key: String,
    pub required: bool,
}

#[derive(Deserialize, Clone, Debug)]
pub struct TopicConfig {
    pub base: String,
    pub separator: String,
    pub key_order: Vec<String>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(tag = "type")]
pub enum ValidationRules {
    StringHandler {
        max_length: Option<usize>,
        required: bool,
    },
    DateHandler {
        canonical_format: String,
        required: bool,
    },
    EnumHandler {
        values: Vec<String>,
        required: bool,
    },
    ExpverHandler {
        default: Option<String>,
        required: bool,
    },
    IntHandler {
        range: Option<[i64; 2]>,
        required: bool,
    },
    TimeHandler {
        required: bool,
    },
}

#[derive(Deserialize, Clone, Debug)]
pub struct EventSchema {
    pub payload: Option<PayloadConfig>,
    pub topic: Option<TopicConfig>,
    pub endpoint: Option<TopicConfig>,
    pub request: HashMap<String, Vec<ValidationRules>>,
}

// LOGGING SETTINGS
#[derive(Deserialize, Clone, Debug)]
pub struct LoggingSettings {
    pub level: String,  // e.g. "info", "debug", "error", "trace", etc.
    pub format: String, // "bunyan", "json", "pretty_json", "console"
}

// NOTIFICATION BACKEND SETTINGS
#[derive(Deserialize, Clone, Debug)]
pub struct NotificationBackendSettings {
    pub kind: String, // The type of notification_backend (e.g., "in_memory", "nats", etc.)
    pub backend_url: Option<String>,
}

// APPLICATION SETTINGS
#[derive(Deserialize, Clone, Debug)]
pub struct Settings {
    pub application: ApplicationSettings,
    pub notification_backend: NotificationBackendSettings,
    pub logging: Option<LoggingSettings>,
    pub notification_schema: Option<HashMap<String, EventSchema>>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct ApplicationSettings {
    pub host: String,
    #[serde(deserialize_with = "deserialize_number_from_string")]
    pub port: u16,
}

pub fn get_configuration() -> Result<Settings, config::ConfigError> {
    let mut settings = config::Config::builder();

    // Config paths for aviso_server
    // ./configuration/config.yaml (relative to current dir)
    // Here the base config is loaded from the current directory
    // and then overridden by the other config files if they exist.
    let base_path = std::env::current_dir().expect("Failed to get current directory");
    let config_dir = base_path.join("configuration").join("config.yaml");
    if config_dir.exists() {
        settings = settings.add_source(config::File::from(config_dir));
    }

    // /etc/aviso_server/config.yaml
    let etc_path = "/etc/aviso_server/config.yaml";
    if std::path::Path::new(etc_path).exists() {
        settings = settings.add_source(config::File::with_name(etc_path).required(false));
    }

    // $HOME/.aviso_server/config.yaml
    if let Some(home_dir) = dirs::home_dir() {
        let user_config_path = home_dir.join(".aviso_server/config.yaml");
        if user_config_path.exists() {
            settings = settings.add_source(config::File::from(user_config_path).required(false));
        }
    }

    // Environment variables
    // The environment variables are prefixed with AVISOSERVER
    settings = settings.add_source(
        config::Environment::with_prefix("AVISOSERVER")
            .prefix_separator("_")
            .separator("__"),
    );

    // Build configuration and deserialize
    let settings = settings.build()?.try_deserialize::<Settings>()?;
    Ok(settings)
}
