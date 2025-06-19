//! HTTP request handling orchestration
//!
//! This module contains functions that orchestrate between different
//! domain modules (cloudevents, notification) and HTTP concerns

pub mod conversion;
pub mod notification_processor;
pub mod storage;
pub mod validation;

pub use conversion::{convert_payload_to_string, get_payload_type_name};
pub use notification_processor::process_notification_request;
pub use storage::save_to_backend;
pub use validation::parse_and_validate_request;
