// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use crate::auth::middleware::{auth_mode, get_user, is_auth_enabled, unauthorized_response};
use crate::configuration::{AuthMode, AuthSettings, Settings};
use crate::notification_backend::replay::StartAt;
use actix_web::{HttpRequest, HttpResponse, web};
use serde_json::json;
use std::sync::Arc;

/// Whether the request is a read (watch/replay) or write (notify) operation.
pub enum StreamOperation {
    Read,
    Write,
}

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

/// Enforce per-stream authentication and authorization.
///
/// Role matching depends on the operation:
/// - **Read**: if `read_roles` is set, user must match; otherwise any authenticated user.
/// - **Write**: if `write_roles` is set, user must match or be admin; otherwise only admins.
pub fn enforce_stream_auth(
    req: &HttpRequest,
    event_type: &str,
    operation: StreamOperation,
) -> Result<(), HttpResponse> {
    if !is_auth_enabled(req) {
        return Ok(());
    }

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

    let Some(user) = get_user(req) else {
        let auth_mode = auth_mode(req).unwrap_or(AuthMode::Direct);
        return Err(unauthorized_response(
            auth_mode,
            "Authentication is required for this stream",
        ));
    };

    let Some(auth_settings) = req.app_data::<web::Data<Arc<AuthSettings>>>() else {
        tracing::error!("AuthSettings not found in app_data — server misconfiguration");
        return Err(HttpResponse::InternalServerError().json(json!({
            "code": "INTERNAL_ERROR",
            "error": "internal_error",
            "message": "Server configuration error"
        })));
    };
    let is_admin = user.is_admin(&auth_settings.admin_roles);

    match operation {
        StreamOperation::Read => {
            if let Some(read_roles) = &stream_auth.read_roles
                && !is_admin
                && !user.has_any_role(read_roles)
            {
                return Err(forbidden_response(
                    "User does not have required read role for this stream",
                ));
            }
            // No read_roles → any authenticated user can read.
        }
        StreamOperation::Write => match &stream_auth.write_roles {
            Some(write_roles) => {
                if !is_admin && !user.has_any_role(write_roles) {
                    return Err(forbidden_response(
                        "User does not have required write role for this stream",
                    ));
                }
            }
            None if !is_admin => {
                return Err(forbidden_response(
                    "Only administrators can write to this stream",
                ));
            }
            None => {}
        },
    }

    Ok(())
}

fn forbidden_response(message: &str) -> HttpResponse {
    HttpResponse::Forbidden().json(json!({
        "code": "FORBIDDEN",
        "error": "forbidden",
        "message": message
    }))
}
