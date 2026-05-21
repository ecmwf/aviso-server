// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use crate::auth::middleware::{auth_mode, get_user, is_auth_enabled, unauthorized_response};
use crate::configuration::{AuthMode, AuthSettings, EventSchema, Settings};
use crate::notification_backend::replay::StartAt;
use actix_web::{HttpRequest, HttpResponse, web};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

/// Whether the request is a read (watch/replay) or write (notify) operation.
pub enum StreamOperation {
    Read,
    Write,
}

/// Reject requests whose `event_type` is not in the configured `notification_schema`
/// when strict mode is active.
///
/// This is the first line of defense against unknown event types: it runs before
/// auth, span recording, metric labels, and any backend work. Returning `Err` short-
/// circuits the request with a 400 response containing the sorted list of
/// configured event types.
///
/// Truth table (strict, schema_present, event_known) → outcome:
///   strict=false, *                        → Ok        (legacy permissive)
///   strict=true,  schema_present=true,  known=true  → Ok
///   strict=true,  schema_present=true,  known=false → Err 400
///   strict=true,  schema_present=false → Err 400 (explicit deny-all "drain" mode)
pub fn enforce_known_event_type(req: &HttpRequest, event_type: &str) -> Result<(), HttpResponse> {
    let request_id = crate::middleware::request_id::request_id_from_request(req);
    enforce_known_event_type_inner(
        Settings::get_global_notification_schema_strict(),
        Settings::get_global_notification_schema().as_ref(),
        &request_id,
        event_type,
    )
}

/// Pure-logic core of `enforce_known_event_type`, factored out so unit tests
/// can drive every cell of the truth table without touching the `OnceLock`
/// globals.
pub(crate) fn enforce_known_event_type_inner(
    strict: bool,
    schema_map: Option<&HashMap<String, EventSchema>>,
    request_id: &str,
    event_type: &str,
) -> Result<(), HttpResponse> {
    if !strict {
        return Ok(());
    }

    let known = schema_map.is_some_and(|m| m.contains_key(event_type));
    if known {
        return Ok(());
    }

    let configured: Vec<String> = schema_map
        .map(|m| {
            let mut names: Vec<String> = m.keys().cloned().collect();
            names.sort();
            names
        })
        .unwrap_or_default();

    Err(HttpResponse::BadRequest().json(json!({
        "code": "UNKNOWN_EVENT_TYPE",
        "error": "unknown_event_type",
        "message": format!("unknown event type '{event_type}'"),
        "configured_event_types": configured,
        "request_id": request_id,
    })))
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

    let request_id = crate::middleware::request_id::request_id_from_request(req);

    let Some(user) = get_user(req) else {
        let auth_mode = auth_mode(req).unwrap_or(AuthMode::Direct);
        return Err(unauthorized_response(
            auth_mode,
            "Authentication is required for this stream",
            &request_id,
        ));
    };

    let Some(auth_settings) = req.app_data::<web::Data<Arc<AuthSettings>>>() else {
        tracing::error!("AuthSettings not found in app_data — server misconfiguration");
        return Err(HttpResponse::InternalServerError().json(json!({
            "code": "INTERNAL_ERROR",
            "error": "internal_error",
            "message": "Server configuration error",
            "request_id": request_id,
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
                    &request_id,
                ));
            }
            // No read_roles → any authenticated user can read.
        }
        StreamOperation::Write => match &stream_auth.write_roles {
            Some(write_roles) if !is_admin && !user.has_any_role(write_roles) => {
                return Err(forbidden_response(
                    "User does not have required write role for this stream",
                    &request_id,
                ));
            }
            Some(_) => {}
            None if !is_admin => {
                return Err(forbidden_response(
                    "Only administrators can write to this stream",
                    &request_id,
                ));
            }
            None => {}
        },
    }

    Ok(())
}

fn forbidden_response(message: &str, request_id: &str) -> HttpResponse {
    HttpResponse::Forbidden().json(json!({
        "code": "FORBIDDEN",
        "error": "forbidden",
        "message": message,
        "request_id": request_id,
    }))
}

#[cfg(feature = "ecpds")]
fn ecpds_service_unavailable_response(request_id: &str) -> HttpResponse {
    HttpResponse::ServiceUnavailable().json(serde_json::json!({
        "code": "SERVICE_UNAVAILABLE",
        "error": "service_unavailable",
        "message": "ECPDS service is inaccessible",
        "request_id": request_id,
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

    let request_id = crate::middleware::request_id::request_id_from_request(req);
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
            username = %user.username,
            error_kind = "missing_auth_settings",
            "AuthSettings not found in app_data — server misconfiguration"
        );
        record_decision("error");
        return Err(HttpResponse::InternalServerError().json(serde_json::json!({
            "code": "INTERNAL_ERROR",
            "error": "internal_error",
            "message": "Server configuration error",
            "request_id": request_id,
        })));
    };

    if user.is_admin(&auth_settings.admin_roles) {
        tracing::debug!(
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

    let Some(checker_data) = req.app_data::<web::Data<Arc<aviso_ecpds::checker::EcpdsChecker>>>()
    else {
        tracing::error!(
            service_name = SERVICE_NAME,
            service_version = SERVICE_VERSION,
            event_name = "auth.ecpds.check.error",
            event_type = %event_type,
            username = %user.username,
            error_kind = "missing_checker",
            "ECPDS plugin referenced but no checker configured"
        );
        record_decision("error");
        return Err(HttpResponse::InternalServerError().json(serde_json::json!({
            "code": "INTERNAL_ERROR",
            "error": "internal_error",
            "message": "Server configuration error",
            "request_id": request_id,
        })));
    };
    let checker: &aviso_ecpds::checker::EcpdsChecker = checker_data.as_ref();

    tracing::debug!(
        service_name = SERVICE_NAME,
        service_version = SERVICE_VERSION,
        event_name = "auth.ecpds.check.started",
        event_type = %event_type,
        username = %user.username,
        "Starting ECPDS destination access check"
    );

    let access = checker
        .check_access(&user.username, canonicalized_params)
        .await;

    let cache_outcome_label = access
        .cache_outcome
        .as_ref()
        .map(|c| c.label())
        .unwrap_or("none");

    if let Some(m) = metrics.as_ref() {
        m.ecpds.cache_size.set(checker.cache_entry_count() as i64);
        // Cache and fetch counters are recorded once per cache lookup,
        // independently of whether the request was allowed, denied, or
        // failed to reach a verdict. fetch_total is incremented only on
        // MissFetched so concurrent waiters that coalesced onto a single
        // upstream call (success or failure) do not over-report.
        if let Some(cache_outcome) = access.cache_outcome {
            match cache_outcome {
                aviso_ecpds::cache::CacheOutcome::Hit => m.ecpds.cache_hits_total.inc(),
                aviso_ecpds::cache::CacheOutcome::MissCoalesced => {
                    m.ecpds.cache_misses_total.inc();
                }
                aviso_ecpds::cache::CacheOutcome::MissFetched { fetch_outcome } => {
                    m.ecpds.cache_misses_total.inc();
                    m.ecpds
                        .fetch_total
                        .with_label_values(&[fetch_outcome.label()])
                        .inc();
                }
            }
        }
    }

    match access.result {
        Ok(()) => {
            tracing::info!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_name = "auth.ecpds.check.allowed",
                event_type = %event_type,
                username = %user.username,
                cache_outcome = cache_outcome_label,
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
                cache_outcome = cache_outcome_label,
                "ECPDS access denied"
            );
            record_decision(reason.label());
            Err(forbidden_response(&message, &request_id))
        }
        Err(aviso_ecpds::EcpdsError::ServiceUnavailable { fetch_outcome }) => {
            tracing::warn!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_name = "auth.ecpds.check.unavailable",
                event_type = %event_type,
                username = %user.username,
                fetch_outcome = ?fetch_outcome,
                cache_outcome = cache_outcome_label,
                "ECPDS service unavailable"
            );
            record_decision("unavailable");
            Err(ecpds_service_unavailable_response(&request_id))
        }
        Err(e) => {
            tracing::error!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_name = "auth.ecpds.check.error",
                event_type = %event_type,
                username = %user.username,
                error = %e,
                cache_outcome = cache_outcome_label,
                "Unexpected ECPDS error"
            );
            record_decision("error");
            Err(HttpResponse::InternalServerError().json(serde_json::json!({
                "code": "INTERNAL_ERROR",
                "error": "internal_error",
                "message": "ECPDS check failed unexpectedly",
                "request_id": request_id,
            })))
        }
    }
}

#[cfg(test)]
mod enforce_known_event_type_tests {
    use super::enforce_known_event_type_inner;
    use crate::configuration::{EventSchema, IdentifierFieldConfig};
    use aviso_validators::ValidationRules;
    use std::collections::HashMap;

    fn schema(event_types: &[&str]) -> HashMap<String, EventSchema> {
        let mut map = HashMap::new();
        for name in event_types {
            map.insert(
                (*name).to_string(),
                EventSchema {
                    payload: None,
                    topic: None,
                    endpoint: None,
                    identifier: HashMap::from([(
                        "class".to_string(),
                        IdentifierFieldConfig::with_rule(ValidationRules::StringHandler {
                            max_length: None,
                            required: true,
                        }),
                    )]),
                    storage_policy: None,
                    auth: None,
                },
            );
        }
        map
    }

    #[test]
    fn non_strict_always_passes_regardless_of_schema() {
        let map = schema(&["mars"]);
        assert!(enforce_known_event_type_inner(false, Some(&map), "req-1", "unknown").is_ok());
        assert!(enforce_known_event_type_inner(false, None, "req-2", "anything").is_ok());
    }

    #[test]
    fn strict_with_known_event_passes() {
        let map = schema(&["mars", "dissemination"]);
        assert!(enforce_known_event_type_inner(true, Some(&map), "req", "mars").is_ok());
        assert!(enforce_known_event_type_inner(true, Some(&map), "req", "dissemination").is_ok());
    }

    #[test]
    fn strict_with_unknown_event_returns_err() {
        let map = schema(&["mars"]);
        let result = enforce_known_event_type_inner(true, Some(&map), "req", "asadasdasd");
        assert!(result.is_err(), "unknown event type must be rejected");
    }

    #[test]
    fn strict_with_no_schema_is_deny_all() {
        let result = enforce_known_event_type_inner(true, None, "req", "anything");
        assert!(
            result.is_err(),
            "strict mode with no schema is a deny-all drain mode, not a permissive bypass"
        );
    }

    #[test]
    fn strict_with_empty_schema_is_deny_all() {
        let map: HashMap<String, EventSchema> = HashMap::new();
        let result = enforce_known_event_type_inner(true, Some(&map), "req", "anything");
        assert!(
            result.is_err(),
            "strict mode with empty schema is a deny-all drain mode"
        );
    }
}
