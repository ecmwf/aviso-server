// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Stateless Streamable HTTP transport for MCP.
//!
//! Every POST carries exactly one JSON-RPC message and gets a plain JSON
//! answer; the server never opens an SSE stream and never issues a
//! session id. Checks run in a fixed order so an unauthenticated caller
//! learns nothing about the body: origin, bearer token, `Accept`,
//! `Content-Type`, protocol version header, then the body itself.

use crate::auth::{User, extract_bearer_token};
use crate::mcp::auth::{TokenError, VerifiedToken};
use crate::mcp::dispatch::{dispatch, methods};
use crate::mcp::protocol::{
    DEFAULT_PROTOCOL_VERSION, Message, RpcError, codes, failure, is_supported_version,
    parse_message, success,
};
use crate::mcp::state::McpState;
use crate::mcp::tools::ToolContext;
use crate::notification_backend::NotificationBackend;
use crate::telemetry::{SERVICE_NAME, SERVICE_VERSION};
use actix_web::http::{StatusCode, header};
use actix_web::{HttpRequest, HttpResponse, HttpResponseBuilder, web};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, info};

/// Largest request body accepted on `/mcp`.
pub const MAX_BODY_BYTES: usize = 64 * 1024;
const PROTOCOL_VERSION_HEADER: &str = "mcp-protocol-version";
const CORS_ALLOW_HEADERS: &str = "authorization, content-type, accept, mcp-protocol-version";

/// What one POST did, for the request log.
struct Outcome {
    response: HttpResponse,
    outcome: &'static str,
    method: Option<String>,
    tool: Option<String>,
    subject_hash: Option<String>,
}

impl Outcome {
    fn new(response: HttpResponse, outcome: &'static str) -> Self {
        Self {
            response,
            outcome,
            method: None,
            tool: None,
            subject_hash: None,
        }
    }
}

/// A POST refused before its message is dispatched.
struct Rejection {
    status: StatusCode,
    outcome: &'static str,
    body: Value,
    challenge: Option<String>,
}

impl Rejection {
    fn new(status: StatusCode, outcome: &'static str, message: &str) -> Self {
        Self::with_body(status, outcome, json!({"error": message}))
    }

    fn with_body(status: StatusCode, outcome: &'static str, body: Value) -> Self {
        Self {
            status,
            outcome,
            body,
            challenge: None,
        }
    }
}

impl From<Rejection> for Outcome {
    fn from(rejection: Rejection) -> Self {
        let mut response = HttpResponseBuilder::new(rejection.status);
        if let Some(challenge) = rejection.challenge {
            response.insert_header((header::WWW_AUTHENTICATE, challenge));
        }
        Outcome::new(response.json(rejection.body), rejection.outcome)
    }
}

/// `POST /mcp`.
pub async fn post_mcp(
    request: HttpRequest,
    payload: web::Payload,
    state: web::Data<McpState>,
    backend: web::Data<Arc<dyn NotificationBackend>>,
) -> HttpResponse {
    let started = Instant::now();
    let origin = match allowed_origin(&request, &state) {
        Ok(origin) => origin,
        Err(()) => {
            let outcome = Outcome::new(origin_denied(), "origin_denied");
            log_outcome(&outcome, started);
            return outcome.response;
        }
    };
    let mut outcome = handle_post(&request, payload, &state, backend.get_ref().clone()).await;
    log_outcome(&outcome, started);
    if let Some(origin) = origin {
        add_cors_headers(&mut outcome.response, &origin);
    }
    outcome.response
}

async fn handle_post(
    request: &HttpRequest,
    payload: web::Payload,
    state: &McpState,
    backend: Arc<dyn NotificationBackend>,
) -> Outcome {
    let verified = match authenticate(request, state).await {
        Ok(verified) => verified,
        Err(rejection) => return rejection.into(),
    };
    let mut outcome = match read_message(request, payload).await {
        Ok(message) => answer(message, &verified.user, state, backend).await,
        Err(rejection) => rejection.into(),
    };
    outcome.subject_hash = Some(verified.subject_hash);
    outcome
}

async fn read_message(request: &HttpRequest, payload: web::Payload) -> Result<Message, Rejection> {
    check_headers(request)?;
    let body = read_body(payload).await?;
    parse_message(&body).map_err(|error| {
        Rejection::with_body(
            StatusCode::BAD_REQUEST,
            "invalid_message",
            failure(None, &error),
        )
    })
}

async fn answer(
    message: Message,
    user: &User,
    state: &McpState,
    backend: Arc<dyn NotificationBackend>,
) -> Outcome {
    let (id, method, params) = match message {
        Message::Request { id, method, params } => (id, method, params),
        Message::Notification { method } => {
            let mut outcome = Outcome::new(HttpResponse::Accepted().finish(), "notification");
            outcome.method = Some(method);
            return outcome;
        }
        Message::Response => {
            return Outcome::new(HttpResponse::Accepted().finish(), "response");
        }
    };
    let context = ToolContext {
        user,
        access: &state.access,
        settings: &state.settings,
        backend,
    };
    let dispatched = dispatch(&context, &method, params).await;
    let (body, label) = match dispatched.result {
        Ok(result) if dispatched.tool_error => (success(&id, result), "tool_error"),
        Ok(result) => (success(&id, result), "ok"),
        Err(error) => (failure(Some(&id), &error), rpc_outcome(&error)),
    };
    let mut outcome = Outcome::new(HttpResponse::Ok().json(body), label);
    outcome.method = Some(method);
    outcome.tool = dispatched.tool;
    outcome
}

fn rpc_outcome(error: &RpcError) -> &'static str {
    match error.code {
        codes::METHOD_NOT_FOUND => "method_not_found",
        codes::INVALID_PARAMS => "invalid_params",
        _ => "rpc_error",
    }
}

/// `Ok(Some(origin))` for an allow-listed browser origin, `Ok(None)` when
/// no `Origin` header was sent, `Err` when the origin is not allowed.
fn allowed_origin(request: &HttpRequest, state: &McpState) -> Result<Option<String>, ()> {
    let Some(origin) = request.headers().get(header::ORIGIN) else {
        return Ok(None);
    };
    let origin = origin.to_str().map_err(|_| ())?;
    if state.settings.allowed_origins.iter().any(|o| o == origin) {
        Ok(Some(origin.to_string()))
    } else {
        Err(())
    }
}

async fn authenticate(request: &HttpRequest, state: &McpState) -> Result<VerifiedToken, Rejection> {
    let bearer = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(extract_bearer_token);
    let Some(token) = bearer else {
        return Err(challenge(
            state,
            StatusCode::UNAUTHORIZED,
            None,
            "unauthenticated",
        ));
    };
    state.verifier.verify(token).await.map_err(|error| {
        debug!(
            service_name = SERVICE_NAME,
            service_version = SERVICE_VERSION,
            event_name = "mcp.auth.rejected",
            reason = %error,
            "MCP bearer token rejected"
        );
        match error {
            TokenError::InsufficientScope => challenge(
                state,
                StatusCode::FORBIDDEN,
                Some("insufficient_scope"),
                "insufficient_scope",
            ),
            TokenError::KeysUnavailable => Rejection::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "keys_unavailable",
                "token verification is temporarily unavailable",
            ),
            _ => challenge(
                state,
                StatusCode::UNAUTHORIZED,
                Some("invalid_token"),
                "invalid_token",
            ),
        }
    })
}

/// RFC 6750 and RFC 9728 challenge pointing at the resource metadata.
///
/// - no token: `Bearer resource_metadata="...", scope="aviso:read"`
/// - bad token: `Bearer error="invalid_token", resource_metadata="...", ...`
fn challenge(
    state: &McpState,
    status: StatusCode,
    error: Option<&str>,
    outcome: &'static str,
) -> Rejection {
    let error_part = error
        .map(|error| format!("error=\"{error}\", "))
        .unwrap_or_default();
    let value = format!(
        "Bearer {error_part}resource_metadata=\"{}\", scope=\"{}\"",
        state.metadata_url,
        state.verifier.required_scope_string()
    );
    let mut rejection = Rejection::new(status, outcome, error.unwrap_or("unauthorized"));
    rejection.challenge = Some(value);
    rejection
}

fn check_headers(request: &HttpRequest) -> Result<(), Rejection> {
    let header_text = |name: &str| {
        request
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
    };
    if !header_text(header::ACCEPT.as_str()).is_some_and(accepts_json) {
        return Err(Rejection::new(
            StatusCode::NOT_ACCEPTABLE,
            "not_acceptable",
            "Accept must include application/json",
        ));
    }
    if !header_text(header::CONTENT_TYPE.as_str()).is_some_and(is_json_media_type) {
        return Err(Rejection::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_media_type",
            "Content-Type must be application/json",
        ));
    }
    let version = header_text(PROTOCOL_VERSION_HEADER).unwrap_or(DEFAULT_PROTOCOL_VERSION);
    if !is_supported_version(version) {
        let error = RpcError::new(
            codes::INVALID_REQUEST,
            format!("Unsupported MCP-Protocol-Version: {version}"),
        );
        return Err(Rejection::with_body(
            StatusCode::BAD_REQUEST,
            "unsupported_version",
            failure(None, &error),
        ));
    }
    Ok(())
}

/// Whether an `Accept` header admits a JSON answer.
///
/// - valid: `application/json, text/event-stream`, `*/*`, `application/*`
/// - invalid: `text/event-stream`, `text/html`
fn accepts_json(accept: &str) -> bool {
    accept.split(',').any(|range| {
        let media = range.split(';').next().unwrap_or_default().trim();
        ["application/json", "application/*", "*/*"]
            .iter()
            .any(|ok| media.eq_ignore_ascii_case(ok))
    })
}

fn is_json_media_type(content_type: &str) -> bool {
    let media = content_type.split(';').next().unwrap_or_default().trim();
    media.eq_ignore_ascii_case("application/json")
}

async fn read_body(mut payload: web::Payload) -> Result<Vec<u8>, Rejection> {
    let mut body = Vec::new();
    while let Some(chunk) = payload.next().await {
        let chunk = chunk.map_err(|_| {
            Rejection::new(
                StatusCode::BAD_REQUEST,
                "unreadable_body",
                "unreadable body",
            )
        })?;
        if body.len() + chunk.len() > MAX_BODY_BYTES {
            return Err(Rejection::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "payload_too_large",
                &format!("body exceeds {MAX_BODY_BYTES} bytes"),
            ));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// `GET`, `DELETE`, and any other method on `/mcp`: this stateless server
/// offers no SSE stream and no sessions.
pub async fn method_not_allowed() -> HttpResponse {
    HttpResponse::MethodNotAllowed()
        .insert_header((header::ALLOW, "POST"))
        .json(json!({"error": "only POST is supported on this endpoint"}))
}

/// `OPTIONS /mcp`: CORS preflight for allow-listed browser origins.
pub async fn preflight(request: HttpRequest, state: web::Data<McpState>) -> HttpResponse {
    match allowed_origin(&request, &state) {
        Ok(Some(origin)) => {
            let mut response = HttpResponse::NoContent()
                .insert_header((header::ACCESS_CONTROL_ALLOW_METHODS, "POST"))
                .insert_header((header::ACCESS_CONTROL_ALLOW_HEADERS, CORS_ALLOW_HEADERS))
                .insert_header((header::ACCESS_CONTROL_MAX_AGE, "600"))
                .finish();
            add_cors_headers(&mut response, &origin);
            response
        }
        Ok(None) => method_not_allowed().await,
        Err(()) => origin_denied(),
    }
}

fn origin_denied() -> HttpResponse {
    HttpResponse::Forbidden().json(json!({"error": "origin not allowed"}))
}

fn add_cors_headers(response: &mut HttpResponse, origin: &str) {
    let headers = response.headers_mut();
    if let Ok(value) = header::HeaderValue::from_str(origin) {
        headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, value);
    }
    headers.insert(header::VARY, header::HeaderValue::from_static("Origin"));
    headers.insert(
        header::ACCESS_CONTROL_EXPOSE_HEADERS,
        header::HeaderValue::from_static("www-authenticate"),
    );
}

/// Method names are client input; only known names reach the logs.
fn known_method(method: &str) -> &'static str {
    methods::ALL
        .iter()
        .copied()
        .find(|known| *known == method)
        .unwrap_or("other")
}

fn log_outcome(outcome: &Outcome, started: Instant) {
    let method = outcome.method.as_deref().map_or("", known_method);
    let tool = outcome.tool.as_deref().unwrap_or("");
    info!(
        service_name = SERVICE_NAME,
        service_version = SERVICE_VERSION,
        event_name = "mcp.request.completed",
        mcp_method = method,
        mcp_tool = tool,
        outcome = outcome.outcome,
        http_status = outcome.response.status().as_u16(),
        duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        "MCP request completed"
    );
    if let Some(subject_hash) = &outcome.subject_hash {
        debug!(
            service_name = SERVICE_NAME,
            service_version = SERVICE_VERSION,
            event_name = "mcp.request.subject",
            subject_hash = %subject_hash,
            mcp_method = method,
            "MCP request subject"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{accepts_json, is_json_media_type};

    #[test]
    fn accept_header_must_admit_json() {
        for ok in [
            "application/json, text/event-stream",
            "text/event-stream, application/json;q=0.9",
            "*/*",
            "Application/JSON",
        ] {
            assert!(accepts_json(ok), "{ok}");
        }
        for bad in ["text/event-stream", "text/html", ""] {
            assert!(!accepts_json(bad), "{bad}");
        }
    }

    #[test]
    fn content_type_must_be_json() {
        assert!(is_json_media_type("application/json; charset=utf-8"));
        assert!(!is_json_media_type("text/plain"));
    }
}
