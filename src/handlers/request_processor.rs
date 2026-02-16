//! Shared request processing logic for streaming endpoints
//!
//! This module provides common validation and processing functionality
//! that can be shared between watch and replay endpoints.

use anyhow::Result;
use std::collections::HashMap;
use tracing_actix_web::RequestId;

use crate::configuration::Settings;
use crate::notification::{NotificationHandler, OperationType};
use crate::notification_backend::replay::StartAt;
use crate::types::NotificationRequest;

/// Context containing all validated request information
#[derive(Debug, Clone)]
pub struct StreamingRequestContext {
    pub event_type: String,
    pub topic: String,
    pub canonicalized_params: HashMap<String, String>,
    pub start_at: StartAt,
    pub request_id: RequestId,
}

/// Configuration for request validation requirements
#[derive(Debug, Clone)]
pub struct ValidationConfig {
    /// Whether replay parameters (from_id or from_date) are required
    pub require_replay_params: bool,
    /// The operation type for schema validation
    pub operation_type: OperationType,
}

impl ValidationConfig {
    /// Create config for watch endpoint (replay params optional)
    pub fn for_watch() -> Self {
        Self {
            require_replay_params: false,
            operation_type: OperationType::Watch,
        }
    }

    /// Create config for replay endpoint (replay params required)
    pub fn for_replay() -> Self {
        Self {
            require_replay_params: true,
            operation_type: OperationType::Replay,
        }
    }
}

/// Shared request processor for streaming endpoints
pub struct StreamingRequestProcessor;

impl StreamingRequestProcessor {
    /// Process and validate a streaming request
    ///
    /// This method handles all common validation logic:
    /// - Parameter validation with configurable requirements
    /// - Schema-based request processing
    /// - Topic generation
    ///
    /// # Arguments
    /// * `request` - The incoming notification request
    /// * `request_id` - Request ID for tracking
    /// * `config` - Validation configuration
    ///
    /// # Returns
    /// * `Ok(StreamingRequestContext)` - Validated request context
    /// * `Err(anyhow::Error)` - Validation failed
    pub fn process_request(
        request: &NotificationRequest,
        request_id: RequestId,
        config: ValidationConfig,
    ) -> Result<StreamingRequestContext> {
        // Validate replay parameters based on configuration
        let start_at = Self::validate_replay_parameters(request, &config)?;

        // Process notification request using schema
        let notification_handler =
            NotificationHandler::from_config(Settings::get_global_notification_schema().as_ref());

        let notification_result = notification_handler.process_request(
            &request.event_type,
            &request.identifier,
            &None, // payload parameter as None since this is for request processing
            config.operation_type,
        )?;

        Ok(StreamingRequestContext {
            event_type: notification_result.event_type,
            topic: notification_result.topic,
            canonicalized_params: notification_result.canonicalized_params,
            start_at,
            request_id,
        })
    }

    /// Validate replay parameters according to endpoint requirements
    fn validate_replay_parameters(
        request: &NotificationRequest,
        config: &ValidationConfig,
    ) -> Result<StartAt> {
        let start_at = request.validate_start_at()?;

        // Check if replay parameters are required but missing
        if config.require_replay_params && matches!(start_at, StartAt::LiveOnly) {
            anyhow::bail!(
                "Replay endpoint requires either from_id or from_date parameter. \
                 Use from_id for sequence-based replay or from_date for time-based replay."
            );
        }

        Ok(start_at)
    }
}
