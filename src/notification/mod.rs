// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Notification validation, canonicalization, and topic routing.

pub mod handler;
pub mod processor;
pub mod registry;
pub mod spatial;
pub mod topic_builder;
pub mod topic_codec;
pub mod topic_parser;
pub mod types;
pub mod wildcard_matcher;

pub use handler::NotificationHandler;
pub use processor::NotificationProcessor;
pub use registry::NotificationRegistry;
pub use topic_codec::{
    decode_subject, decode_subject_base, decode_subject_for_display, decode_token, encode_subject,
    encode_token,
};
pub use types::{IdentifierConstraint, OperationType, ProcessingResult};
pub use wildcard_matcher::{analyze_watch_pattern, matches_watch_pattern};
