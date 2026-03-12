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
pub use validation::{
    validate_auth_settings, validate_schema_storage_policy_support, validate_stream_auth_settings,
};
