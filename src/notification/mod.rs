//! Notification processing module for Aviso server
//!
//! This module provides comprehensive notification request validation, canonicalization,
//! and topic building based on configurable schemas. It supports both schema-driven
//! validation for known event types and generic fallback processing for unknown types.

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
    decode_subject, decode_subject_base, decode_token, encode_subject, encode_token,
};
pub use types::{OperationType, ProcessingResult};
pub use wildcard_matcher::{analyze_watch_pattern, matches_watch_pattern};
