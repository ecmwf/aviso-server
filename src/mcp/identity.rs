// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Roles of an MCP user, read from the claims of a verified token.

use serde_json::{Map, Value};
use thiserror::Error;

/// Most roles taken from one token. Tokens are capped at 16 KiB, so this
/// mainly guards against a claim padded with tiny entries.
pub const MAX_ROLES: usize = 256;
/// Longest role accepted, in characters.
pub const MAX_ROLE_CHARS: usize = 128;
/// Claim that carries the roles.
const ROLES_CLAIM: &str = "roles";

/// Why the identity claims of a token were refused.
///
/// Messages describe the rule that failed and never echo claim values,
/// so they are safe to send back to the client.
#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum IdentityClaimError {
    /// The roles claim is not an array of strings.
    #[error("roles claim is not an array of strings")]
    RolesType,
    /// The roles claim has more than [`MAX_ROLES`] entries.
    #[error("roles claim has more than {MAX_ROLES} entries")]
    TooManyRoles,
    /// A role is empty, too long, padded, or has control characters.
    #[error(
        "roles claim entries must be 1 to {MAX_ROLE_CHARS} characters \
         without surrounding whitespace or control characters"
    )]
    RoleFormat,
}

/// Roles from the optional `roles` claim. A token without it, or with
/// `"roles": null`, has none.
pub fn token_roles(claims: &Map<String, Value>) -> Result<Vec<String>, IdentityClaimError> {
    claims
        .get(ROLES_CLAIM)
        .filter(|value| !value.is_null())
        .map(parse_roles)
        .transpose()
        .map(Option::unwrap_or_default)
}

/// Parse a roles claim value.
///
/// - valid: `[]`, `["aviso-mcp", "reader"]`
/// - invalid: `"reader"`, `[1]`, `[""]`, `[" reader"]`, 257 entries
fn parse_roles(value: &Value) -> Result<Vec<String>, IdentityClaimError> {
    let entries = value.as_array().ok_or(IdentityClaimError::RolesType)?;
    if entries.len() > MAX_ROLES {
        return Err(IdentityClaimError::TooManyRoles);
    }
    entries
        .iter()
        .map(|entry| {
            let role = entry.as_str().ok_or(IdentityClaimError::RolesType)?;
            if is_valid_role(role) {
                Ok(role.to_string())
            } else {
                Err(IdentityClaimError::RoleFormat)
            }
        })
        .collect()
}

/// Whether one role entry is usable. Roles are matched exactly against
/// the configured role lists, which have no surrounding whitespace.
///
/// - valid: `reader`, `aviso-mcp`, `data team`, `*`
/// - invalid: `` (empty), ` `, ` reader`, `reader `, `a\nb`, 129 letters
fn is_valid_role(role: &str) -> bool {
    !role.is_empty()
        && role.trim() == role
        && role.chars().count() <= MAX_ROLE_CHARS
        && !role.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn claims(value: Value) -> Map<String, Value> {
        match value {
            Value::Object(map) => map,
            other => panic!("claims must be an object, got {other}"),
        }
    }

    #[test]
    fn roles_are_optional_and_may_be_empty() {
        assert_eq!(token_roles(&claims(json!({}))), Ok(Vec::new()));
        assert_eq!(token_roles(&claims(json!({"roles": null}))), Ok(Vec::new()));
        assert_eq!(token_roles(&claims(json!({"roles": []}))), Ok(Vec::new()));
        assert_eq!(
            token_roles(&claims(json!({"roles": ["aviso-mcp", "reader"]}))),
            Ok(vec!["aviso-mcp".to_string(), "reader".to_string()])
        );
    }

    #[test]
    fn roles_of_the_wrong_type_are_refused() {
        for roles in [json!("reader"), json!([1]), json!([null]), json!({"r": 1})] {
            assert_eq!(
                token_roles(&claims(json!({"roles": roles}))),
                Err(IdentityClaimError::RolesType),
                "{roles}"
            );
        }
    }

    #[test]
    fn roles_out_of_bounds_are_refused() {
        let many = json!(vec!["r"; MAX_ROLES + 1]);
        assert_eq!(
            token_roles(&claims(json!({"roles": many}))),
            Err(IdentityClaimError::TooManyRoles)
        );
        let at_limit = json!(vec!["r"; MAX_ROLES]);
        assert!(token_roles(&claims(json!({"roles": at_limit}))).is_ok());
        let long = "r".repeat(MAX_ROLE_CHARS + 1);
        for role in [
            "",
            " ",
            " reader",
            "reader\u{a0}",
            "role\nwith-newline",
            "tab\there",
            long.as_str(),
        ] {
            assert_eq!(
                token_roles(&claims(json!({"roles": [role]}))),
                Err(IdentityClaimError::RoleFormat),
                "{role:?}"
            );
        }
        let longest = "r".repeat(MAX_ROLE_CHARS);
        let inner_space = json!({"roles": [longest, "data team", "*"]});
        assert!(token_roles(&claims(inner_space)).is_ok());
    }
}
