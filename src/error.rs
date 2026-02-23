//! Generic error handling utilities for the Aviso server
//!
//! This module provides reusable error handling functionality that can be
//! used across different endpoints, validation types, and processing modules.

use crate::notification::decode_subject_for_display;
use actix_web::HttpResponse;
use serde_json::json;
use tracing::{error, warn};

/// Extension trait for converting anyhow errors to HTTP responses
///
/// This trait provides a consistent way to convert any processing errors
/// into appropriate HTTP responses with detailed error information for debugging.
pub trait ToHttpResponse {
    /// Convert an error into an HTTP response with structured error details
    ///
    /// # Returns
    /// * `HttpResponse` - BadRequest response with error chain details
    fn to_http_response(self) -> HttpResponse;

    /// Convert an error into an HTTP response with custom error type
    ///
    /// # Arguments
    /// * `error_type` - The type of error (e.g., "Invalid CloudEvent", "Validation Failed")
    ///
    /// # Returns
    /// * `HttpResponse` - BadRequest response with custom error type
    fn to_http_response_with_type(self, error_type: &str) -> HttpResponse;
}

impl ToHttpResponse for anyhow::Error {
    fn to_http_response(self) -> HttpResponse {
        self.to_http_response_with_type("Processing failed")
    }

    fn to_http_response_with_type(self, error_type: &str) -> HttpResponse {
        let error_chain = extract_error_chain(&self);

        warn!(
            error_chain = ?error_chain,
            error_type = error_type,
            "Request processing failed"
        );

        HttpResponse::BadRequest().json(json!({
            "error": error_type,
            "message": error_chain.first().unwrap_or(&self.to_string()),
            "details": error_chain.last().unwrap_or(&self.to_string()),
            "error_chain": error_chain
        }))
    }
}

/// Extract the full error chain from an anyhow error
///
/// This walks through the error chain to collect all error messages,
/// providing comprehensive debugging information.
///
/// # Arguments
/// * `error` - The anyhow error to extract the chain from
///
/// # Returns
/// * `Vec<String>` - All error messages in the chain from top-level to root cause
fn extract_error_chain(error: &anyhow::Error) -> Vec<String> {
    let mut error_chain = Vec::new();
    let mut current_error: &dyn std::error::Error = error.as_ref();

    loop {
        error_chain.push(current_error.to_string());
        match current_error.source() {
            Some(source) => current_error = source,
            None => break,
        }
    }

    error_chain
}

/// Create a standardized validation error response
///
/// This is a convenience function for validation-specific errors that provides
/// consistent error formatting across different validation types.
///
/// # Arguments
/// * `validation_type` - The type of validation that failed (e.g., "CloudEvent")
/// * `error` - The validation error
///
/// # Returns
/// * `HttpResponse` - BadRequest response formatted for validation errors
pub fn validation_error_response(validation_type: &str, error: anyhow::Error) -> HttpResponse {
    error.to_http_response_with_type(&format!("Invalid {}", validation_type))
}

/// Create a standardized processing error response
///
/// This is a convenience function for processing-specific errors.
///
/// # Arguments
/// * `process_type` - The type of processing that failed (e.g., "CloudEvent", "Notification")
/// * `error` - The processing error
///
/// # Returns
/// * `HttpResponse` - InternalServerError response formatted for processing errors
pub fn processing_error_response(process_type: &str, error: anyhow::Error) -> HttpResponse {
    let error_chain = extract_error_chain(&error);

    error!(
        error_chain = ?error_chain,
        process_type = process_type,
        "Processing failed"
    );

    HttpResponse::InternalServerError().json(json!({
        "error": format!("{} processing failed", process_type),
        "message": error_chain.first().unwrap_or(&error.to_string()),
        "details": error_chain.last().unwrap_or(&error.to_string()),
        "error_chain": error_chain
    }))
}

/// Create a standardized SSE error response
///
/// This is a convenience function for SSE-specific errors.
///
/// # Arguments
/// * `error` - The SSE error
/// * `topic` - The topic that failed
/// * `request_id` - The request ID for tracking
///
/// # Returns
/// * `HttpResponse` - InternalServerError response formatted for SSE errors
pub fn sse_error_response(error: anyhow::Error, topic: &str, request_id: &str) -> HttpResponse {
    let error_chain = extract_error_chain(&error);
    let display_topic = decode_subject_for_display(topic);

    error!(
        error_chain = ?error_chain,
        topic = display_topic,
        request_id = request_id,
        "SSE stream creation failed"
    );

    HttpResponse::InternalServerError().json(json!({
        "error": "SSE stream creation failed",
        "message": error_chain.first().unwrap_or(&error.to_string()),
        "details": error_chain.last().unwrap_or(&error.to_string()),
        "topic": display_topic,
        "request_id": request_id,
        "error_chain": error_chain
    }))
}
