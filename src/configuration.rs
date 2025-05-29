use dirs;
use serde::{Deserialize, Serialize};
use serde_aux::field_attributes::deserialize_number_from_string;
use std::collections::HashMap;
use std::sync::OnceLock;

// NOTIFICATION SETTINGS
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct PayloadConfig {
    pub key: String,
    pub required: bool,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct TopicConfig {
    pub base: String,
    pub separator: String,
    pub key_order: Vec<String>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
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

impl ValidationRules {
    /// Check if this validation rule marks a field as required
    pub fn is_required(&self) -> bool {
        match self {
            ValidationRules::StringHandler { required, .. } => *required,
            ValidationRules::DateHandler { required, .. } => *required,
            ValidationRules::EnumHandler { required, .. } => *required,
            ValidationRules::ExpverHandler { required, .. } => *required,
            ValidationRules::IntHandler { required, .. } => *required,
            ValidationRules::TimeHandler { required } => *required,
        }
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct EventSchema {
    pub payload: Option<PayloadConfig>,
    pub topic: Option<TopicConfig>,
    pub endpoint: Option<TopicConfig>,
    pub request: HashMap<String, Vec<ValidationRules>>,
}

// LOGGING SETTINGS
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct LoggingSettings {
    pub level: String,  // e.g. "info", "debug", "error", "trace", etc.
    pub format: String, // "bunyan", "json", "pretty_json", "console"
}

// NOTIFICATION BACKEND SETTINGS
#[derive(serde::Deserialize, Serialize, Clone, Debug)]
pub struct JetStreamSettings {
    pub nats_url: Option<String>,
    pub token: Option<String>,
    pub timeout_seconds: Option<u64>,
    pub retry_attempts: Option<u32>,
    pub max_messages: Option<i64>,
    pub max_bytes: Option<i64>,
    pub retention_days: Option<u32>,
    pub storage_type: Option<String>,
    pub replicas: Option<usize>,
    pub retention_policy: Option<String>,
    pub discard_policy: Option<String>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct NotificationBackendSettings {
    pub kind: String,
    // Backend-specific configurations
    #[serde(default)]
    pub jetstream: Option<JetStreamSettings>,
    // Future backends can be added here
    // pub kafka: Option<KafkaSettings>,
}

// APPLICATION SETTINGS
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct ApplicationSettings {
    pub host: String,
    #[serde(deserialize_with = "deserialize_number_from_string")]
    pub port: u16,
}

// MAIN SETTINGS STRUCT
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Settings {
    pub application: ApplicationSettings,
    pub notification_backend: NotificationBackendSettings,
    pub logging: Option<LoggingSettings>,
    pub notification_schema: Option<HashMap<String, EventSchema>>,
}

// GLOBAL CONFIGURATION MANAGEMENT
// ================================
/// Global notification schema storage using OnceLock for thread-safe initialization
///
/// OnceLock provides:
/// - Thread-safe initialization (only one thread can initialize it)
/// - Zero-cost access after initialization (just a pointer dereference)
/// - Compile-time guarantee that it's initialized before use
/// - No runtime overhead for synchronization after initialization
static GLOBAL_NOTIFICATION_SCHEMA: OnceLock<Option<HashMap<String, EventSchema>>> = OnceLock::new();

/// Global logging settings storage
static GLOBAL_LOGGING_SETTINGS: OnceLock<Option<LoggingSettings>> = OnceLock::new();

impl Settings {
    /// Initialize global configuration components
    ///
    /// This method extracts frequently-accessed configuration parts and stores them
    /// in global static variables for efficient access throughout the application.
    pub fn init_global_config(&self) {
        // Initialize notification schema globally
        // This is accessed on every notification request, so global access provides
        // significant performance benefits
        let _ = GLOBAL_NOTIFICATION_SCHEMA.set(self.notification_schema.clone());

        // Initialize logging settings globally
        // Logging configuration is accessed frequently by the tracing system
        let _ = GLOBAL_LOGGING_SETTINGS.set(self.logging.clone());

        tracing::info!(
            has_notification_schema = self.notification_schema.is_some(),
            has_logging_config = self.logging.is_some(),
            "Global configuration initialized successfully"
        );
    }

    /// Get reference to the global notification schema
    ///
    /// # Returns
    /// Reference to the notification schema HashMap, or None if no schema configured
    ///
    /// # Panic
    /// Panics if called before `init_global_config()`. This is intentional to catch
    /// programming errors early rather than returning a Result.
    pub fn get_global_notification_schema() -> &'static Option<HashMap<String, EventSchema>> {
        GLOBAL_NOTIFICATION_SCHEMA
            .get()
            .expect("Global notification schema not initialized. Call Settings::init_global_config() first.")
    }

    /// Get reference to the global logging settings
    ///
    /// # Returns
    /// Reference to the logging settings, or None if no logging configured
    ///
    /// # Panic
    /// Panics if called before `init_global_config()`
    pub fn get_global_logging_settings() -> &'static Option<LoggingSettings> {
        GLOBAL_LOGGING_SETTINGS.get().expect(
            "Global logging settings not initialized. Call Settings::init_global_config() first.",
        )
    }

    /// Check if global configuration has been initialized
    ///
    /// Useful for testing or conditional initialization logic.
    pub fn is_global_config_initialized() -> bool {
        GLOBAL_NOTIFICATION_SCHEMA.get().is_some() && GLOBAL_LOGGING_SETTINGS.get().is_some()
    }
}

// CONFIGURATION LOADING
// =====================
/// Load configuration from multiple sources with proper precedence
///
/// Configuration sources in order of precedence (later sources override earlier ones):
/// 1. ./configuration/config.yaml (base configuration)
/// 2. /etc/aviso_server/config.yaml (system-wide configuration)
/// 3. $HOME/.aviso_server/config.yaml (user-specific configuration)
/// 4. Environment variables (highest precedence)
pub fn get_configuration() -> Result<Settings, config::ConfigError> {
    let mut settings = config::Config::builder();

    // Base configuration from current directory
    // This is typically used during development and contains sensible defaults
    let base_path = std::env::current_dir().expect("Failed to get current directory");
    let config_dir = base_path.join("configuration").join("config.yaml");
    if config_dir.exists() {
        tracing::debug!(path = ?config_dir, "Loading base configuration");
        settings = settings.add_source(config::File::from(config_dir));
    }

    // System-wide configuration
    // Used in production deployments for shared settings across all users
    let etc_path = "/etc/aviso_server/config.yaml";
    if std::path::Path::new(etc_path).exists() {
        tracing::debug!(path = etc_path, "Loading system configuration");
        settings = settings.add_source(config::File::with_name(etc_path).required(false));
    }

    // User-specific configuration
    // Allows individual users to customize settings without affecting others
    if let Some(home_dir) = dirs::home_dir() {
        let user_config_path = home_dir.join(".aviso_server/config.yaml");
        if user_config_path.exists() {
            tracing::debug!(path = ?user_config_path, "Loading user configuration");
            settings = settings.add_source(config::File::from(user_config_path).required(false));
        }
    }

    // Environment variables (highest precedence)
    // Perfect for container deployments and CI/CD pipelines
    // Format: AVISOSERVER_APPLICATION__HOST=127.0.0.1
    //         AVISOSERVER_NOTIFICATION_BACKEND__KIND=nats
    settings = settings.add_source(
        config::Environment::with_prefix("AVISOSERVER")
            .prefix_separator("_")
            .separator("__"),
    );

    // Build and validate the final configuration
    let settings = settings.build()?.try_deserialize::<Settings>()?;

    tracing::info!(
        host = %settings.application.host,
        port = settings.application.port,
        backend_kind = %settings.notification_backend.kind,
        has_notification_schema = settings.notification_schema.is_some(),
        "Configuration loaded successfully"
    );

    Ok(settings)
}
