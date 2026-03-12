use crate::auth::middleware::{auth_mode, get_user, is_auth_enabled, unauthorized_response};
use crate::configuration::{AuthMode, Settings};
use crate::notification_backend::replay::StartAt;
use actix_web::{HttpRequest, HttpResponse};
use serde_json::json;

pub fn record_start_at_span_fields(start_at: StartAt) {
    match start_at {
        StartAt::Sequence(id) => {
            tracing::Span::current().record("from_id", id);
        }
        StartAt::Date(date) => {
            tracing::Span::current().record("from_date", date.to_rfc3339());
        }
        StartAt::LiveOnly => {}
    }
}

pub fn enforce_stream_auth(req: &HttpRequest, event_type: &str) -> Result<(), HttpResponse> {
    // Middleware marks whether auth is globally enabled for this request.
    if !is_auth_enabled(req) {
        return Ok(());
    }

    // Stream auth is keyed by schema event_type, not topic base.
    let stream_auth = Settings::get_global_notification_schema()
        .as_ref()
        .and_then(|schema_map| schema_map.get(event_type))
        .and_then(|schema| schema.auth.as_ref());

    let Some(stream_auth) = stream_auth else {
        return Ok(());
    };

    if !stream_auth.required {
        return Ok(());
    }

    // Required stream auth without a user means the request is unauthenticated.
    let Some(user) = get_user(req) else {
        // Middleware always sets auth mode when enabled; default is defensive
        // for non-standard call paths (for example direct unit invocation).
        let auth_mode = auth_mode(req).unwrap_or(AuthMode::Direct);
        return Err(unauthorized_response(
            auth_mode,
            "Authentication is required for this stream",
        ));
    };

    if let Some(allowed_roles) = &stream_auth.allowed_roles
        && !allowed_roles.is_empty()
        && !user.has_any_role(allowed_roles)
    {
        // User is authenticated, but lacks stream authorization.
        return Err(HttpResponse::Forbidden().json(json!({
            "code": "FORBIDDEN",
            "error": "forbidden",
            "message": "User does not have required role for this stream"
        })));
    }

    Ok(())
}
