use crate::configuration::LoggingSettings;
use tracing::level_filters::LevelFilter;
use tracing::{Subscriber, subscriber::set_global_default};
use tracing_bunyan_formatter::{BunyanFormattingLayer, JsonStorageLayer};
use tracing_log::LogTracer;
use tracing_subscriber::{EnvFilter, Layer, Registry, fmt, fmt::MakeWriter, layer::SubscriberExt};

pub const SERVICE_NAME: &str = env!("CARGO_PKG_NAME");
pub const SERVICE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Build a tracing subscriber with configurable log level and output format.
/// # Parameters
/// - `name`: Name of the app (used in Bunyan logs)
/// - `logging_config`: Optional logging settings (level, format)
/// - `sink`: Destination for log output (e.g., stdout, file)
pub fn get_subscriber<Sink>(
    name: String,
    logging_config: Option<&LoggingSettings>,
    sink: Sink,
) -> impl Subscriber + Sync + Send
where
    Sink: for<'a> MakeWriter<'a> + Send + Sync + 'static,
{
    // Determine log level from config or default to "info"
    let level = logging_config
        .map(|config| config.level.clone())
        .unwrap_or_else(|| "info".to_string());

    let level_filter = match level.to_lowercase().as_str() {
        "trace" => LevelFilter::TRACE,
        "debug" => LevelFilter::DEBUG,
        "warn" => LevelFilter::WARN,
        "error" => LevelFilter::ERROR,
        // default to INFO
        _ => LevelFilter::INFO,
    };
    // Set up filter layer for log level filtering
    let filter_layer = EnvFilter::default().add_directive(level_filter.into());

    // Determine log output format from config or default to "console"
    let formatting_format = logging_config
        .map(|config| config.format.clone())
        .unwrap_or_else(|| "console".to_string());

    // Select formatting layer based on format
    let formatting_layer = match formatting_format.to_lowercase().as_str() {
        "json" => Box::new(fmt::layer().json()) as Box<dyn Layer<_> + Send + Sync + 'static>,
        "compact" => Box::new(fmt::layer().compact()) as Box<dyn Layer<_> + Send + Sync + 'static>,
        "bunyan" => {
            // BunyanFormattingLayer provides structured JSON logs
            Box::new(BunyanFormattingLayer::new(name, sink))
                as Box<dyn Layer<_> + Send + Sync + 'static>
        }
        // default to console
        _ => Box::new(fmt::layer().pretty()) as Box<dyn Layer<_> + Send + Sync + 'static>,
    };

    // Combine all layers: filter, JSON storage (for spans), formatting
    Registry::default()
        .with(filter_layer)
        .with(JsonStorageLayer)
        .with(formatting_layer)
}

/// Register the given subscriber as the global default for tracing/log events.
///
/// This should only be called ONCE, typically at the start of your main function.
///
/// - Bridges the legacy `log` crate to `tracing` via LogTracer, so all logs are unified.
/// - Panics if called multiple times or on error.
pub fn init_subscriber(subscriber: impl Subscriber + Sync + Send) {
    // Enables all events from the `log` crate (used by many libraries) to be
    // processed by tracing (and thus included in your logs).
    LogTracer::init().expect("Failed to set logger");

    // Sets this subscriber as the global default for the application.
    // Must be called only once! Subsequent calls will panic.
    set_global_default(subscriber).expect("Failed to set subscriber");
}
