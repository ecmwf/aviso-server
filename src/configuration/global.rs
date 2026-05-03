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

#[cfg(feature = "ecpds")]
static GLOBAL_ECPDS_CHECKER: OnceLock<Option<aviso_ecpds::checker::EcpdsChecker>> = OnceLock::new();

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

        #[cfg(feature = "ecpds")]
        {
            let checker = self
                .ecpds
                .as_ref()
                .map(aviso_ecpds::checker::EcpdsChecker::new);
            let _ = GLOBAL_ECPDS_CHECKER.set(checker);
        }

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

    #[cfg(feature = "ecpds")]
    pub fn get_global_ecpds_checker() -> &'static Option<aviso_ecpds::checker::EcpdsChecker> {
        GLOBAL_ECPDS_CHECKER.get().expect(
            "Global ECPDS checker not initialized. Call Settings::init_global_config() first.",
        )
    }
}
