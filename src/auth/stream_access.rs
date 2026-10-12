// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Transport-independent per-stream access decisions.
//!
//! The REST routes and any other entry point share this logic so the rules
//! for who may read or write a stream are defined in one place. Callers map
//! the returned [`StreamAccessError`] onto their own response format.

use crate::auth::User;
use crate::configuration::{Settings, StreamAuthConfig};
use std::collections::HashMap;
use thiserror::Error;

/// Whether the request is a read (watch/replay) or write (notify) operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamOperation {
    Read,
    Write,
}

/// Reason a stream access check denied the request.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum StreamAccessError {
    /// The stream requires authentication and no user is present.
    #[error("Authentication is required for this stream")]
    Unauthenticated,
    /// The admin role mapping is unavailable, so no decision can be made.
    #[error("Server configuration error")]
    MissingAdminRoles,
    /// The user lacks a role listed in the stream's `read_roles`.
    #[error("User does not have required read role for this stream")]
    MissingReadRole,
    /// The user lacks a role listed in the stream's `write_roles`.
    #[error("User does not have required write role for this stream")]
    MissingWriteRole,
    /// The stream has no `write_roles`, so only admins may write.
    #[error("Only administrators can write to this stream")]
    AdminOnlyWrite,
}

/// Decide whether `user` may perform `operation` on `event_type`.
///
/// The stream's auth block is read from the global notification schema.
/// `admin_roles` is `None` when the caller could not resolve the auth
/// settings; that only matters once a user must be checked against roles.
///
/// Role matching depends on the operation:
/// - **Read**: if `read_roles` is set, user must match or be admin;
///   otherwise any authenticated user.
/// - **Write**: if `write_roles` is set, user must match or be admin;
///   otherwise only admins.
pub fn check_stream_access(
    auth_enabled: bool,
    user: Option<&User>,
    admin_roles: Option<&HashMap<String, Vec<String>>>,
    event_type: &str,
    operation: StreamOperation,
) -> Result<(), StreamAccessError> {
    if !auth_enabled {
        return Ok(());
    }
    let stream_auth = Settings::get_global_notification_schema()
        .as_ref()
        .and_then(|schema_map| schema_map.get(event_type))
        .and_then(|schema| schema.auth.as_ref());
    check_stream_access_with(stream_auth, user, admin_roles, operation)
}

/// Pure core of [`check_stream_access`] that takes the stream auth block
/// directly, so tests can cover every rule without global configuration.
pub(crate) fn check_stream_access_with(
    stream_auth: Option<&StreamAuthConfig>,
    user: Option<&User>,
    admin_roles: Option<&HashMap<String, Vec<String>>>,
    operation: StreamOperation,
) -> Result<(), StreamAccessError> {
    let Some(stream_auth) = stream_auth.filter(|auth| auth.required) else {
        return Ok(());
    };
    let user = user.ok_or(StreamAccessError::Unauthenticated)?;
    let admin_roles = admin_roles.ok_or(StreamAccessError::MissingAdminRoles)?;
    let is_admin = user.is_admin(admin_roles);

    match operation {
        StreamOperation::Read => match &stream_auth.read_roles {
            Some(read_roles) if !is_admin && !user.has_any_role(read_roles) => {
                Err(StreamAccessError::MissingReadRole)
            }
            _ => Ok(()),
        },
        StreamOperation::Write => match &stream_auth.write_roles {
            Some(write_roles) if !is_admin && !user.has_any_role(write_roles) => {
                Err(StreamAccessError::MissingWriteRole)
            }
            Some(_) => Ok(()),
            None if !is_admin => Err(StreamAccessError::AdminOnlyWrite),
            None => Ok(()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roles(pairs: &[(&str, &[&str])]) -> HashMap<String, Vec<String>> {
        pairs
            .iter()
            .map(|(realm, r)| (realm.to_string(), r.iter().map(|s| s.to_string()).collect()))
            .collect()
    }

    fn user(realm: &str, user_roles: &[&str]) -> User {
        User {
            username: "u".to_string(),
            realm: Some(realm.to_string()),
            roles: user_roles.iter().map(|s| s.to_string()).collect(),
            attributes: HashMap::new(),
        }
    }

    fn auth(
        read: Option<HashMap<String, Vec<String>>>,
        write: Option<HashMap<String, Vec<String>>>,
    ) -> StreamAuthConfig {
        StreamAuthConfig {
            required: true,
            read_roles: read,
            write_roles: write,
            plugins: None,
        }
    }

    fn admins() -> HashMap<String, Vec<String>> {
        roles(&[("ecmwf", &["admin"])])
    }

    #[test]
    fn auth_disabled_allows_everything() {
        let result = check_stream_access(false, None, None, "any-stream", StreamOperation::Write);
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn missing_or_optional_stream_auth_allows_anonymous() {
        let admin = admins();
        let ok = check_stream_access_with(None, None, Some(&admin), StreamOperation::Read);
        assert_eq!(ok, Ok(()));
        let mut optional = auth(None, None);
        optional.required = false;
        let ok =
            check_stream_access_with(Some(&optional), None, Some(&admin), StreamOperation::Write);
        assert_eq!(ok, Ok(()));
    }

    #[test]
    fn anonymous_user_is_unauthenticated() {
        let cfg = auth(None, None);
        let result =
            check_stream_access_with(Some(&cfg), None, Some(&admins()), StreamOperation::Read);
        assert_eq!(result, Err(StreamAccessError::Unauthenticated));
    }

    #[test]
    fn anonymous_check_runs_before_admin_roles_lookup() {
        let cfg = auth(None, None);
        let result = check_stream_access_with(Some(&cfg), None, None, StreamOperation::Read);
        assert_eq!(result, Err(StreamAccessError::Unauthenticated));
        let u = user("ecmwf", &["reader"]);
        let result = check_stream_access_with(Some(&cfg), Some(&u), None, StreamOperation::Read);
        assert_eq!(result, Err(StreamAccessError::MissingAdminRoles));
    }

    #[test]
    fn read_without_read_roles_allows_any_authenticated_user() {
        let cfg = auth(None, None);
        let u = user("other", &[]);
        let result =
            check_stream_access_with(Some(&cfg), Some(&u), Some(&admins()), StreamOperation::Read);
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn wrong_realm_is_denied_read() {
        let cfg = auth(Some(roles(&[("ecmwf", &["reader"])])), None);
        let u = user("desp", &["reader"]);
        let result =
            check_stream_access_with(Some(&cfg), Some(&u), Some(&admins()), StreamOperation::Read);
        assert_eq!(result, Err(StreamAccessError::MissingReadRole));
    }

    #[test]
    fn wildcard_grants_read_to_whole_realm() {
        let cfg = auth(Some(roles(&[("mcp", &["*"])])), None);
        let u = user("mcp", &["anything"]);
        let result =
            check_stream_access_with(Some(&cfg), Some(&u), Some(&admins()), StreamOperation::Read);
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn admin_bypasses_read_and_write_roles() {
        let cfg = auth(
            Some(roles(&[("other", &["reader"])])),
            Some(roles(&[("other", &["writer"])])),
        );
        let u = user("ecmwf", &["admin"]);
        let admin = admins();
        for op in [StreamOperation::Read, StreamOperation::Write] {
            let result = check_stream_access_with(Some(&cfg), Some(&u), Some(&admin), op);
            assert_eq!(result, Ok(()));
        }
    }

    #[test]
    fn write_roles_match_or_deny() {
        let cfg = auth(None, Some(roles(&[("ecmwf", &["writer"])])));
        let admin = admins();
        let writer = user("ecmwf", &["writer"]);
        let reader = user("ecmwf", &["reader"]);
        let ok = check_stream_access_with(
            Some(&cfg),
            Some(&writer),
            Some(&admin),
            StreamOperation::Write,
        );
        assert_eq!(ok, Ok(()));
        let denied = check_stream_access_with(
            Some(&cfg),
            Some(&reader),
            Some(&admin),
            StreamOperation::Write,
        );
        assert_eq!(denied, Err(StreamAccessError::MissingWriteRole));
    }

    #[test]
    fn write_without_write_roles_is_admin_only() {
        let cfg = auth(None, None);
        let admin = admins();
        let reader = user("ecmwf", &["reader"]);
        let denied = check_stream_access_with(
            Some(&cfg),
            Some(&reader),
            Some(&admin),
            StreamOperation::Write,
        );
        assert_eq!(denied, Err(StreamAccessError::AdminOnlyWrite));
        let boss = user("ecmwf", &["admin"]);
        let ok = check_stream_access_with(
            Some(&cfg),
            Some(&boss),
            Some(&admin),
            StreamOperation::Write,
        );
        assert_eq!(ok, Ok(()));
    }
}
