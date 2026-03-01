//! HTTP request handling orchestration
//!
//! This module contains functions that orchestrate between different
//! domain modules (cloudevents, notification) and HTTP concerns

pub mod notification_processor;
pub mod request_processor;
pub mod storage;
pub mod validation;

pub use notification_processor::{
    NotificationErrorKind, NotificationProcessingError, process_notification_request,
};
pub use request_processor::{StreamingRequestContext, StreamingRequestProcessor, ValidationConfig};
pub use storage::save_to_backend;
pub use validation::{RequestParseError, parse_and_validate_request};
