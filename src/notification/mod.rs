//! Notification processing module for Aviso server
//!
//! This module provides comprehensive notification request validation, canonicalization,
//! and topic building based on configurable schemas. It supports both schema-driven
//! validation for known event types and generic fallback processing for unknown types.

pub mod handler;
pub mod processor;
pub mod registry;
pub mod topic_builder;
pub mod types;
pub mod validators;

pub use handler::NotificationHandler;
pub use processor::NotificationProcessor;
pub use registry::NotificationRegistry;
pub use types::{OperationType, ProcessingResult};
