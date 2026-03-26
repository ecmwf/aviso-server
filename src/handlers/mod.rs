// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

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
