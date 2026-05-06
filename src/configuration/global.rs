// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use super::{ApplicationSettings, EventSchema, LoggingSettings, Settings, WatchEndpointSettings};
use crate::telemetry::{SERVICE_NAME, SERVICE_VERSION};
use std::collections::HashMap;
use std::sync::OnceLock;

static GLOBAL_NOTIFICATION_SCHEMA: OnceLock<Option<HashMap<String, EventSchema>>> = OnceLock::new();
static GLOBAL_LOGGING_SETTINGS: OnceLock<Option<LoggingSettings>> = OnceLock::new();
static GLOBAL_APPLICATION_SETTINGS: OnceLock<ApplicationSettings> = OnceLock::new();
static GLOBAL_WATCH_SETTINGS: OnceLock<WatchEndpointSettings> = OnceLock::new();

impl Settings {
    /// Stores read-mostly config in global immutable slots.
    ///
    /// Invariant: call once during startup before any request handling path reads
    /// global settings (for example schema lookups in notification processing).
    pub fn init_global_config(&self) {
        let _ = GLOBAL_NOTIFICATION_SCHEMA.set(self.notification_schema.clone());
        let _ = GLOBAL_LOGGING_SETTINGS.set(self.logging.clone());
        let _ = GLOBAL_APPLICATION_SETTINGS.set(self.application.clone());
        let _ = GLOBAL_WATCH_SETTINGS.set(self.watch_endpoint.clone());

        tracing::info!(
            service_name = SERVICE_NAME,
            service_version = SERVICE_VERSION,
            event_name = "configuration.global.initialized",
            has_notification_schema = self.notification_schema.is_some(),
            has_logging_config = self.logging.is_some(),
            base_url = %self.application.base_url,
            "Global configuration initialized successfully"
        );
    }

    /// Build the per-application ECPDS checker (if configured).
    ///
    /// Returned to the caller (typically `Application::build`) so it
    /// can be flowed into actix `app_data` and read by route handlers.
    /// One instance per running Aviso process; tests may build their
    /// own per-app instance to exercise distinct ECPDS configurations
    /// (e.g. different `partial_outage_policy` or server lists) within
    /// a single test binary.
    ///
    /// Returns `Ok(None)` (no checker built) when *either* there is
    /// no `ecpds:` config block *or* no stream actually enables the
    /// `"ecpds"` plugin. The opt-in is per stream, and constructing
    /// the checker for an unused but stale `ecpds:` block would let
    /// a malformed `servers` URL or a bad credential break startup
    /// for a deployment that does not actually authorise any stream
    /// through ECPDS, which contradicts the per-stream opt-in story.
    ///
    /// Returns an error if checker construction fails (e.g. invalid
    /// server URL or HTTP client builder error) for a config block
    /// that *is* referenced by at least one stream.
    #[cfg(feature = "ecpds")]
    pub fn build_ecpds_checker(
        &self,
    ) -> Result<Option<aviso_ecpds::checker::EcpdsChecker>, aviso_ecpds::EcpdsError> {
        let any_stream_enables_ecpds = self
            .notification_schema
            .as_ref()
            .map(|schema| {
                schema.iter().any(|(_, event_schema)| {
                    event_schema
                        .auth
                        .as_ref()
                        .and_then(|a| a.plugins.as_ref())
                        .map(|plugins| plugins.iter().any(|p| p == "ecpds"))
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false);
        let checker = match (self.ecpds.as_ref(), any_stream_enables_ecpds) {
            (Some(cfg), true) => Some(aviso_ecpds::checker::EcpdsChecker::new(cfg)?),
            _ => None,
        };
        tracing::info!(
            service_name = SERVICE_NAME,
            service_version = SERVICE_VERSION,
            event_name = "configuration.ecpds.initialized",
            ecpds_block_present = self.ecpds.is_some(),
            any_stream_enables_ecpds,
            checker_built = checker.is_some(),
            "ECPDS checker initialized"
        );
        Ok(checker)
    }

    pub fn get_global_notification_schema() -> &'static Option<HashMap<String, EventSchema>> {
        GLOBAL_NOTIFICATION_SCHEMA
            .get()
            .expect("Global notification schema not initialized. Call Settings::init_global_config() first.")
    }

    /// Panics before startup initialization; this catches invalid init order.
    pub fn get_global_logging_settings() -> &'static Option<LoggingSettings> {
        GLOBAL_LOGGING_SETTINGS.get().expect(
            "Global logging settings not initialized. Call Settings::init_global_config() first.",
        )
    }

    /// Panics before startup initialization; this catches invalid init order.
    pub fn get_global_watch_settings() -> &'static WatchEndpointSettings {
        GLOBAL_WATCH_SETTINGS.get().expect(
            "Global watch settings not initialized. Call Settings::init_global_config() first.",
        )
    }

    /// Panics before startup initialization; this catches invalid init order.
    pub fn get_global_application_settings() -> &'static ApplicationSettings {
        GLOBAL_APPLICATION_SETTINGS
            .get()
            .expect("Global application settings not initialized. Call Settings::init_global_config() first.")
    }

    pub fn is_global_config_initialized() -> bool {
        GLOBAL_NOTIFICATION_SCHEMA.get().is_some()
            && GLOBAL_LOGGING_SETTINGS.get().is_some()
            && GLOBAL_APPLICATION_SETTINGS.get().is_some()
            && GLOBAL_WATCH_SETTINGS.get().is_some()
    }
}
