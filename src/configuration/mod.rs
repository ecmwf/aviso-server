// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Configuration module split by concern:
//! - `types`: serde-facing config structs
//! - `loader`: source precedence and deserialization
//! - `global`: read-mostly global snapshots used at runtime
//! - `units`: strict parsers for duration/size config literals
mod auth;
mod global;
mod loader;
mod types;
mod units;
mod validation;

pub use auth::{AuthMode, AuthSettings};
pub use loader::get_configuration;
pub use types::*;
pub use units::{parse_duration_spec, parse_retention_time_spec, parse_size_spec};
#[cfg(feature = "ecpds")]
pub use validation::validate_ecpds_settings;
pub use validation::{
    validate_auth_settings, validate_metrics_settings, validate_schema_storage_policy_support,
    validate_stream_auth_settings, validate_stream_plugin_settings,
};
