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
//! admins skip those checks too. MCP callers are not admins.

use crate::auth::User;
use crate::auth::stream_access::{StreamOperation, check_stream_access, stream_requires_plugins};
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

    #[test]
    fn mcp_callers_are_not_admins() {
        let user = User {
            username: "u".to_string(),
            realm: Some("mcp".to_string()),
            roles: vec!["admin".to_string()],
            attributes: HashMap::new(),
        };
        assert!(!McpAccess::default().is_admin(&user));
    }
}
