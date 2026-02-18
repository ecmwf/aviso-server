//! HTTP-facing error model for the Aviso API.

use crate::handlers::RequestParseError;
use crate::notification::decode_subject_for_display;
use crate::telemetry::{SERVICE_NAME, SERVICE_VERSION};
use actix_web::{HttpResponse, ResponseError, http::StatusCode};
use serde_json::json;
use thiserror::Error;
use tracing::{error, warn};

/// Stable machine-readable API error codes.
///
/// These values are part of the external HTTP contract and should be treated
/// as stable once released.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiErrorCode {
    InvalidJson,
    UnknownField,
    InvalidRequestShape,
    InvalidNotificationRequest,
    InvalidWatchRequest,
    InvalidReplayRequest,
    NotificationProcessingFailed,
    NotificationStorageFailed,
    SseStreamInitializationFailed,
    InternalError,
}

impl ApiErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ApiErrorCode::InvalidJson => "INVALID_JSON",
            ApiErrorCode::UnknownField => "UNKNOWN_FIELD",
            ApiErrorCode::InvalidRequestShape => "INVALID_REQUEST_SHAPE",
            ApiErrorCode::InvalidNotificationRequest => "INVALID_NOTIFICATION_REQUEST",
            ApiErrorCode::InvalidWatchRequest => "INVALID_WATCH_REQUEST",
            ApiErrorCode::InvalidReplayRequest => "INVALID_REPLAY_REQUEST",
            ApiErrorCode::NotificationProcessingFailed => "NOTIFICATION_PROCESSING_FAILED",
            ApiErrorCode::NotificationStorageFailed => "NOTIFICATION_STORAGE_FAILED",
            ApiErrorCode::SseStreamInitializationFailed => "SSE_STREAM_INITIALIZATION_FAILED",
            ApiErrorCode::InternalError => "INTERNAL_ERROR",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum RequestKind {
    Notification,
    Watch,
    Replay,
}

impl RequestKind {
    // Request kind drives a single 4xx code family per endpoint to keep client
    // handling predictable even when validation rules evolve.
    fn code(self) -> ApiErrorCode {
        match self {
            RequestKind::Notification => ApiErrorCode::InvalidNotificationRequest,
            RequestKind::Watch => ApiErrorCode::InvalidWatchRequest,
            RequestKind::Replay => ApiErrorCode::InvalidReplayRequest,
        }
    }

    fn label(self) -> &'static str {
        match self {
            RequestKind::Notification => "Invalid Notification Request",
            RequestKind::Watch => "Invalid Watch Request",
            RequestKind::Replay => "Invalid Replay Request",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ProcessingKind {
    NotificationProcessing,
    NotificationStorage,
}

impl ProcessingKind {
    fn code(self) -> ApiErrorCode {
        match self {
            ProcessingKind::NotificationProcessing => ApiErrorCode::NotificationProcessingFailed,
            ProcessingKind::NotificationStorage => ApiErrorCode::NotificationStorageFailed,
        }
    }

    fn label(self) -> &'static str {
        match self {
            ProcessingKind::NotificationProcessing => "Notification Processing Failed",
            ProcessingKind::NotificationStorage => "Notification Storage Failed",
        }
    }
}

#[derive(Debug, Error)]
pub enum ApiError {
    // Parse errors are syntactic/shape failures before domain validation.
    #[error("{error}")]
    Parse {
        code: ApiErrorCode,
        error: &'static str,
        #[source]
        source: RequestParseError,
    },
    #[error("{error}")]
    Validation {
        code: ApiErrorCode,
        error: &'static str,
        #[source]
        source: anyhow::Error,
    },
    #[error("{error}")]
    Processing {
        code: ApiErrorCode,
        error: &'static str,
        #[source]
        source: anyhow::Error,
    },
    #[error("SSE stream creation failed")]
    Sse {
        code: ApiErrorCode,
        topic: String,
        request_id: String,
        #[source]
        source: anyhow::Error,
    },
}

impl ApiError {
    // Parse error codes are determined by the parser variant, never by message text.
    fn parse(kind: RequestKind, source: RequestParseError) -> Self {
        let code = match source {
            RequestParseError::InvalidJson(_) => ApiErrorCode::InvalidJson,
            RequestParseError::UnknownField(_) => ApiErrorCode::UnknownField,
            RequestParseError::InvalidShape(_) => ApiErrorCode::InvalidRequestShape,
        };

        Self::Parse {
            code,
            error: kind.label(),
            source,
        }
    }

    fn validation(kind: RequestKind, source: anyhow::Error) -> Self {
        Self::Validation {
            code: kind.code(),
            error: kind.label(),
            source,
        }
    }

    fn processing(kind: ProcessingKind, source: anyhow::Error) -> Self {
        Self::Processing {
            code: kind.code(),
            error: kind.label(),
            source,
        }
    }

    fn sse(topic: &str, request_id: &str, source: anyhow::Error) -> Self {
        Self::Sse {
            code: ApiErrorCode::SseStreamInitializationFailed,
            topic: topic.to_string(),
            request_id: request_id.to_string(),
            source,
        }
    }
}

impl ResponseError for ApiError {
    fn status_code(&self) -> StatusCode {
        match self {
            ApiError::Parse { .. } | ApiError::Validation { .. } => StatusCode::BAD_REQUEST,
            ApiError::Processing { .. } | ApiError::Sse { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn error_response(&self) -> HttpResponse {
        match self {
            ApiError::Parse {
                code,
                error: error_label,
                source,
            } => {
                let message = source.to_string();
                warn!(
                    service_name = SERVICE_NAME,
                    service_version = SERVICE_VERSION,
                    event_domain = "http",
                    event_name = "api_request_parse_failed",
                    outcome = "error",
                    error_code = code.as_str(),
                    error = %error_label,
                    details = %message,
                    "Request parsing failed"
                );
                HttpResponse::build(self.status_code()).json(json!({
                    "code": code.as_str(),
                    "error": error_label,
                    "message": message,
                    "details": message,
                }))
            }
            ApiError::Validation {
                code,
                error: error_label,
                source,
            } => {
                let (message, details, chain) = error_summary(source);
                warn!(
                    service_name = SERVICE_NAME,
                    service_version = SERVICE_VERSION,
                    event_domain = "http",
                    event_name = "api_request_validation_failed",
                    outcome = "error",
                    error_code = code.as_str(),
                    error = %error_label,
                    error_chain = ?chain,
                    "Request validation failed"
                );
                HttpResponse::build(self.status_code()).json(json!({
                    "code": code.as_str(),
                    "error": error_label,
                    "message": message,
                    "details": details,
                }))
            }
            ApiError::Processing {
                code,
                error: error_label,
                source,
            } => {
                let (message, details, chain) = error_summary(source);
                error!(
                    service_name = SERVICE_NAME,
                    service_version = SERVICE_VERSION,
                    event_domain = "http",
                    event_name = "api_request_processing_failed",
                    outcome = "error",
                    error_code = code.as_str(),
                    error = %error_label,
                    error_chain = ?chain,
                    "Request processing failed"
                );
                HttpResponse::build(self.status_code()).json(json!({
                    "code": code.as_str(),
                    "error": error_label,
                    "message": message,
                    "details": details,
                }))
            }
            ApiError::Sse {
                code,
                topic,
                request_id,
                source,
            } => {
                let (message, details, chain) = error_summary(source);
                let display_topic = decode_subject_for_display(topic);
                error!(
                    service_name = SERVICE_NAME,
                    service_version = SERVICE_VERSION,
                    event_domain = "streaming",
                    event_name = "api_sse_stream_initialization_failed",
                    outcome = "error",
                    error_code = code.as_str(),
                    error_chain = ?chain,
                    topic = display_topic,
                    request_id = request_id,
                    "SSE stream creation failed"
                );
                HttpResponse::build(self.status_code()).json(json!({
                    "code": code.as_str(),
                    "error": "SSE stream creation failed",
                    "message": message,
                    "details": details,
                    "topic": display_topic,
                    "request_id": request_id,
                }))
            }
        }
    }
}

fn error_summary(error: &anyhow::Error) -> (String, String, Vec<String>) {
    let chain = error
        .chain()
        .map(ToString::to_string)
        .collect::<Vec<String>>();
    let message = chain
        .first()
        .cloned()
        .unwrap_or_else(|| "Unknown error".to_string());
    let details = chain.last().cloned().unwrap_or_else(|| message.clone());
    (message, details, chain)
}

pub fn request_parse_error_response(kind: RequestKind, error: RequestParseError) -> HttpResponse {
    ApiError::parse(kind, error).error_response()
}

pub fn request_validation_error_response(kind: RequestKind, error: anyhow::Error) -> HttpResponse {
    ApiError::validation(kind, error).error_response()
}

pub fn processing_error_response(kind: ProcessingKind, error: anyhow::Error) -> HttpResponse {
    ApiError::processing(kind, error).error_response()
}

pub fn sse_error_response(error: anyhow::Error, topic: &str, request_id: &str) -> HttpResponse {
    ApiError::sse(topic, request_id, error).error_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::RequestParseError;
    use actix_web::body::to_bytes;
    use anyhow::anyhow;
    use serde_json::{Value, json};

    async fn response_json(response: HttpResponse) -> Value {
        let body = response.into_body();
        let bytes = to_bytes(body)
            .await
            .expect("response body should be readable");
        serde_json::from_slice(&bytes).expect("response should be valid json")
    }

    #[test]
    fn parse_error_uses_specific_code_for_json() {
        let parse_error = RequestParseError::InvalidJson(
            serde_json::from_slice::<serde_json::Value>(b"{").expect_err("must fail"),
        );

        let response = ApiError::parse(RequestKind::Replay, parse_error).error_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn validation_error_maps_to_bad_request() {
        let response =
            ApiError::validation(RequestKind::Watch, anyhow!("from_id must be positive"))
                .error_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn processing_error_maps_to_internal_server_error() {
        let response = ApiError::processing(
            ProcessingKind::NotificationStorage,
            anyhow!("failed to write to backend"),
        )
        .error_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn sse_error_maps_to_internal_server_error() {
        let response = ApiError::sse("test.topic", "request-1", anyhow!("stream setup failed"))
            .error_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[actix_web::test]
    async fn parse_error_body_has_stable_shape_and_code() {
        let parse_error = RequestParseError::InvalidJson(
            serde_json::from_slice::<serde_json::Value>(b"{").expect_err("must fail"),
        );
        let response = request_parse_error_response(RequestKind::Replay, parse_error);
        let json = response_json(response).await;

        assert_eq!(json["code"], "INVALID_JSON");
        assert_eq!(json["error"], "Invalid Replay Request");
        assert!(json["message"].is_string());
        assert!(json["details"].is_string());
    }

    #[actix_web::test]
    async fn parse_error_maps_unknown_field_and_shape_codes() {
        let unknown_field_response = request_parse_error_response(
            RequestKind::Watch,
            RequestParseError::UnknownField(anyhow!("Unknown field 'foo' in request")),
        );
        let unknown_field_json = response_json(unknown_field_response).await;
        assert_eq!(unknown_field_json["code"], "UNKNOWN_FIELD");

        let invalid_shape_source =
            serde_json::from_value::<std::collections::HashMap<String, String>>(json!(1))
                .expect_err("must fail to create invalid shape error");
        let invalid_shape_response = request_parse_error_response(
            RequestKind::Notification,
            RequestParseError::InvalidShape(invalid_shape_source),
        );
        let invalid_shape_json = response_json(invalid_shape_response).await;
        assert_eq!(invalid_shape_json["code"], "INVALID_REQUEST_SHAPE");
    }

    #[actix_web::test]
    async fn validation_error_body_has_stable_shape() {
        let response = request_validation_error_response(
            RequestKind::Watch,
            anyhow!("Cannot specify both identifier.polygon and point"),
        );
        let json = response_json(response).await;

        assert_eq!(json["code"], "INVALID_WATCH_REQUEST");
        assert_eq!(json["error"], "Invalid Watch Request");
        assert!(json["message"].is_string());
        assert!(json["details"].is_string());
        assert!(json.get("error_chain").is_none());
    }

    #[actix_web::test]
    async fn processing_and_sse_errors_have_expected_contract() {
        let processing_response = processing_error_response(
            ProcessingKind::NotificationStorage,
            anyhow!("backend write failed"),
        );
        let processing_json = response_json(processing_response).await;
        assert_eq!(processing_json["code"], "NOTIFICATION_STORAGE_FAILED");
        assert_eq!(processing_json["error"], "Notification Storage Failed");

        let sse_response =
            sse_error_response(anyhow!("stream setup failed"), "mars.ens%2Emember", "req-1");
        let sse_json = response_json(sse_response).await;
        assert_eq!(sse_json["code"], "SSE_STREAM_INITIALIZATION_FAILED");
        assert_eq!(sse_json["error"], "SSE stream creation failed");
        assert_eq!(sse_json["request_id"], "req-1");
        assert_eq!(sse_json["topic"], "mars.ens.member");
    }

    #[test]
    fn api_error_code_strings_are_stable() {
        assert_eq!(ApiErrorCode::InvalidJson.as_str(), "INVALID_JSON");
        assert_eq!(
            ApiErrorCode::InvalidReplayRequest.as_str(),
            "INVALID_REPLAY_REQUEST"
        );
        assert_eq!(
            ApiErrorCode::SseStreamInitializationFailed.as_str(),
            "SSE_STREAM_INITIALIZATION_FAILED"
        );
    }
}
