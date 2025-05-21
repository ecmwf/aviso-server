use tracing::Subscriber;
use tracing::subscriber::set_global_default;
use tracing_bunyan_formatter::{BunyanFormattingLayer, JsonStorageLayer};
use tracing_log::LogTracer;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::{EnvFilter, Registry, layer::SubscriberExt};

/// Build a tracing subscriber that outputs Bunyan (JSON) logs, with configurable filter and sink.
/// # Parameters
/// - `name`: Name of the app (will appear as "name" field in logs)
/// - `env_filter`: Logging level filter (e.g., "info")
///     - Can be overridden at runtime by setting the `RUST_LOG` environment variable.
/// - `sink`: Where logs should be written (stdout, file, etc.)
pub fn get_subscriber<Sink>(
    name: String,
    env_filter: String,
    sink: Sink,
) -> impl Subscriber + Sync + Send
where
    Sink: for<'a> MakeWriter<'a> + Send + Sync + 'static,
{
    // Allow dynamic log level configuration via RUST_LOG environment variable.
    // Falls back to the provided `env_filter` if not set.
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(env_filter));

    // BunyanFormattingLayer gives structured JSON logs, suitable for log aggregation.
    let formatting_layer = BunyanFormattingLayer::new(name, sink);

    // Registry combines all layers: filter, JSON storage (for spans), formatting.
    Registry::default()
        .with(env_filter)
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
