// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Stream access rules for MCP callers.
//!
//! MCP callers always authenticate and are never admins, so every check
//! runs with authentication on and an empty admin role map. Streams that
//! need an auth plugin are unreadable, because plugin checks need the REST
//! request context.

use crate::auth::User;
use crate::auth::stream_access::{StreamOperation, check_stream_access, stream_requires_plugins};
use crate::sse::collect::ReplayAccess;
use std::collections::HashMap;
use std::sync::LazyLock;

/// Admin role map for MCP callers: nobody is an admin.
static NO_ADMIN_ROLES: LazyLock<HashMap<String, Vec<String>>> = LazyLock::new(HashMap::new);

/// Whether `user` may read `event_type` over MCP.
pub fn mcp_can_read(user: &User, event_type: &str) -> bool {
    !stream_requires_plugins(event_type)
        && check_stream_access(
            true,
            Some(user),
            Some(&NO_ADMIN_ROLES),
            event_type,
            StreamOperation::Read,
        )
        .is_ok()
}

/// Replay access for `user` under the MCP rules.
pub fn mcp_replay_access(user: &User) -> ReplayAccess<'_> {
    ReplayAccess {
        auth_enabled: true,
        user: Some(user),
        admin_roles: Some(&NO_ADMIN_ROLES),
    }
}
