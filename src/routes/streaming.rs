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
            Some(write_roles) if !is_admin && !user.has_any_role(write_roles) => {
                return Err(forbidden_response(
                    "User does not have required write role for this stream",
                ));
            }
            Some(_) => {}
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

#[cfg(feature = "ecpds")]
fn ecpds_service_unavailable_response() -> HttpResponse {
    HttpResponse::ServiceUnavailable().json(serde_json::json!({
        "code": "SERVICE_UNAVAILABLE",
        "error": "service_unavailable",
        "message": "ECPDS service is unaccessible"
    }))
}

/// Enforce ECPDS destination-based authorization for read operations.
///
/// Called AFTER `process_request()` so canonicalized_params are available.
/// Only runs if the stream schema has `plugins: ["ecpds"]` in its auth
/// config. Admins bypass this check.
///
/// All decisions emit structured tracing events under the
/// `auth.ecpds.check.*` namespace and increment the corresponding
/// outcome label on `aviso_ecpds_access_decisions_total`. Cache hits
/// and misses additionally increment `aviso_ecpds_cache_hits_total` /
/// `aviso_ecpds_cache_misses_total`.
#[cfg(feature = "ecpds")]
pub async fn enforce_ecpds_auth(
    req: &HttpRequest,
    event_type: &str,
    canonicalized_params: &std::collections::HashMap<String, String>,
) -> Result<(), HttpResponse> {
    use crate::metrics::AppMetrics;
    use crate::telemetry::{SERVICE_NAME, SERVICE_VERSION};

    let has_ecpds_plugin = Settings::get_global_notification_schema()
        .as_ref()
        .and_then(|schema_map| schema_map.get(event_type))
        .and_then(|schema| schema.auth.as_ref())
        .and_then(|auth| auth.plugins.as_ref())
        .map(|plugins| plugins.iter().any(|p| p == "ecpds"))
        .unwrap_or(false);

    if !has_ecpds_plugin {
        return Ok(());
    }

    let Some(user) = get_user(req) else {
        return Ok(());
    };

    let metrics = req.app_data::<web::Data<AppMetrics>>().cloned();
    let record_decision = |outcome: &str| {
        if let Some(m) = metrics.as_ref() {
            m.ecpds
                .access_decisions_total
                .with_label_values(&[outcome])
                .inc();
        }
    };

    let Some(auth_settings) = req.app_data::<web::Data<Arc<AuthSettings>>>() else {
        tracing::error!(
            service_name = SERVICE_NAME,
            service_version = SERVICE_VERSION,
            event_name = "auth.ecpds.check.error",
            event_type = %event_type,
            error_kind = "missing_auth_settings",
            "AuthSettings not found in app_data — server misconfiguration"
        );
        record_decision("error");
        return Err(HttpResponse::InternalServerError().json(serde_json::json!({
            "code": "INTERNAL_ERROR",
            "error": "internal_error",
            "message": "Server configuration error"
        })));
    };

    if user.is_admin(&auth_settings.admin_roles) {
        tracing::info!(
            service_name = SERVICE_NAME,
            service_version = SERVICE_VERSION,
            event_name = "auth.ecpds.admin.bypass",
            event_type = %event_type,
            username = %user.username,
            "Admin user bypassing ECPDS check"
        );
        record_decision("admin_bypass");
        return Ok(());
    }

    let Some(checker) = Settings::get_global_ecpds_checker().as_ref() else {
        tracing::error!(
            service_name = SERVICE_NAME,
            service_version = SERVICE_VERSION,
            event_name = "auth.ecpds.check.error",
            event_type = %event_type,
            error_kind = "missing_checker",
            "ECPDS plugin referenced but no checker configured"
        );
        record_decision("error");
        return Err(HttpResponse::InternalServerError().json(serde_json::json!({
            "code": "INTERNAL_ERROR",
            "error": "internal_error",
            "message": "Server configuration error"
        })));
    };

    tracing::debug!(
        service_name = SERVICE_NAME,
        service_version = SERVICE_VERSION,
        event_name = "auth.ecpds.check.started",
        event_type = %event_type,
        username = %user.username,
        "Starting ECPDS destination access check"
    );

    let result = checker
        .check_access(&user.username, canonicalized_params)
        .await;

    if let Some(m) = metrics.as_ref() {
        m.ecpds.cache_size.set(checker.cache_entry_count() as i64);
    }

    let record_fetch = |outcome_label: &str| {
        if let Some(m) = metrics.as_ref() {
            m.ecpds
                .fetch_total
                .with_label_values(&[outcome_label])
                .inc();
        }
    };

    match result {
        Ok(cache_outcome) => {
            if let Some(m) = metrics.as_ref() {
                match cache_outcome {
                    aviso_ecpds::cache::CacheOutcome::Hit => m.ecpds.cache_hits_total.inc(),
                    aviso_ecpds::cache::CacheOutcome::Miss => {
                        m.ecpds.cache_misses_total.inc();
                        record_fetch(aviso_ecpds::client::FetchOutcome::Success.label());
                    }
                }
            }
            tracing::info!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_name = "auth.ecpds.check.allowed",
                event_type = %event_type,
                username = %user.username,
                cache_outcome = ?cache_outcome,
                "ECPDS access allowed"
            );
            record_decision("allow");
            Ok(())
        }
        Err(aviso_ecpds::EcpdsError::AccessDenied { reason, message }) => {
            tracing::warn!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_name = "auth.ecpds.check.denied",
                event_type = %event_type,
                username = %user.username,
                reason = ?reason,
                "ECPDS access denied"
            );
            record_decision(reason.label());
            Err(forbidden_response(&message))
        }
        Err(aviso_ecpds::EcpdsError::ServiceUnavailable { fetch_outcome }) => {
            tracing::warn!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_name = "auth.ecpds.check.unavailable",
                event_type = %event_type,
                username = %user.username,
                fetch_outcome = ?fetch_outcome,
                "ECPDS service unavailable"
            );
            record_decision("unavailable");
            record_fetch(fetch_outcome.label());
            Err(ecpds_service_unavailable_response())
        }
        Err(e) => {
            let outcome = e.fetch_outcome();
            tracing::error!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_name = "auth.ecpds.check.error",
                event_type = %event_type,
                username = %user.username,
                error = %e,
                "Unexpected ECPDS error"
            );
            record_decision("error");
            record_fetch(outcome.label());
            Err(HttpResponse::InternalServerError().json(serde_json::json!({
                "code": "INTERNAL_ERROR",
                "error": "internal_error",
                "message": "ECPDS check failed unexpectedly"
            })))
        }
    }
}
