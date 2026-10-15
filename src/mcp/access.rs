// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Stream access rules for MCP callers.
//!
//! MCP callers always authenticate, so every check runs with
//! authentication on and uses the same read rule as REST watch and
//! replay: stream `read_roles` per realm, `"*"` for a whole realm, and an
//! admin bypass. Streams that need an auth plugin are readable only by
//! admins, because plugin checks need the REST request context; REST
//! admins skip those checks too.
//!
//! Nobody is an admin unless the identity source is `token_claims` with
//! `admin: true`. Then the REST `auth.admin_roles` apply to the realms
//! and roles from the token. A fixed MCP realm is never an admin realm.

use crate::auth::User;
use crate::auth::stream_access::{StreamOperation, check_stream_access, stream_requires_plugins};
use crate::configuration::McpIdentitySettings;
use crate::sse::collect::ReplayAccess;
use std::collections::HashMap;

/// Read access decisions for MCP callers.
#[derive(Debug, Clone, Default)]
pub struct McpAccess {
    /// Realm to admin roles. Empty means no admins, see
    /// [`User::is_admin`].
    admin_roles: HashMap<String, Vec<String>>,
}

impl McpAccess {
    /// Rules for users from `identity`. `admin_roles` is the REST
    /// `auth.admin_roles` map; it is used only when `identity` opts in.
    pub fn new(identity: &McpIdentitySettings, admin_roles: &HashMap<String, Vec<String>>) -> Self {
        if identity.grants_admin() {
            // The state outlives the configuration it is built from, so
            // it keeps its own copy of the map.
            Self {
                admin_roles: admin_roles.clone(),
            }
        } else {
            Self::default()
        }
    }

    /// Whether `user` may read `event_type`.
    pub fn can_read(&self, user: &User, event_type: &str) -> bool {
        let allowed = check_stream_access(
            true,
            Some(user),
            Some(&self.admin_roles),
            event_type,
            StreamOperation::Read,
        )
        .is_ok();
        allowed && (!stream_requires_plugins(event_type) || self.is_admin(user))
    }

    /// Replay access for `user` under the same rules.
    pub fn replay_access<'a>(&'a self, user: &'a User) -> ReplayAccess<'a> {
        ReplayAccess {
            auth_enabled: true,
            user: Some(user),
            admin_roles: Some(&self.admin_roles),
        }
    }

    fn is_admin(&self, user: &User) -> bool {
        user.is_admin(&self.admin_roles)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn token_claims(admin: bool) -> McpIdentitySettings {
        serde_json::from_value(json!({"source": "token_claims", "admin": admin})).unwrap()
    }

    fn admins() -> HashMap<String, Vec<String>> {
        HashMap::from([
            ("ecmwf".to_string(), vec!["admin".to_string()]),
            ("mcp".to_string(), vec!["admin".to_string()]),
        ])
    }

    fn user(realm: &str, role: &str) -> User {
        User {
            username: "u".to_string(),
            realm: Some(realm.to_string()),
            roles: vec![role.to_string()],
            attributes: HashMap::new(),
        }
    }

    #[test]
    fn token_claims_with_admin_use_the_rest_admin_roles() {
        let access = McpAccess::new(&token_claims(true), &admins());
        assert!(access.is_admin(&user("ecmwf", "admin")));
        assert!(!access.is_admin(&user("ecmwf", "reader")));
        assert!(!access.is_admin(&user("other", "admin")));
    }

    #[test]
    fn token_claims_without_admin_grant_no_admin() {
        let access = McpAccess::new(&token_claims(false), &admins());
        assert!(!access.is_admin(&user("ecmwf", "admin")));
    }

    #[test]
    fn fixed_identities_are_never_admins() {
        let access = McpAccess::new(&McpIdentitySettings::default(), &admins());
        assert!(!access.is_admin(&user("mcp", "admin")));
    }

    #[test]
    fn empty_admin_role_map_makes_nobody_an_admin() {
        let access = McpAccess::new(&token_claims(true), &HashMap::new());
        assert!(!access.is_admin(&user("ecmwf", "admin")));
    }
}
