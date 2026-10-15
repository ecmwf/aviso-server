// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! `mcp.identity`: where the realm and roles of an MCP user come from.
//!
//! The source is an explicit choice:
//!
//! ```yaml
//! identity:
//!   source: fixed          # one configured realm for every MCP user
//!   realm: "mcp"
//! ```

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Longest realm accepted, in characters.
pub const MCP_REALM_MAX_CHARS: usize = 64;

/// `mcp.identity`: how a verified token becomes an Aviso user.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum McpIdentitySettings {
    /// Every MCP user gets `realm`; roles come from the token's optional
    /// `roles` claim.
    Fixed {
        /// Realm matched against stream `read_roles`. Every stream whose
        /// `read_roles` names it is readable by MCP tokens with a
        /// matching role, so pick a realm name used for nothing else.
        #[serde(default = "default_fixed_realm")]
        realm: String,
    },
}

impl Default for McpIdentitySettings {
    fn default() -> Self {
        Self::Fixed {
            realm: default_fixed_realm(),
        }
    }
}

fn default_fixed_realm() -> String {
    "mcp".to_string()
}

/// A rule the `mcp.identity` block violates.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum McpIdentityError {
    /// The fixed `realm` is not a valid realm name.
    #[error(
        "mcp.identity.realm must be 1 to {MCP_REALM_MAX_CHARS} characters \
         of A-Z, a-z, 0-9, '.', '_' or '-'"
    )]
    Realm,
}

impl McpIdentitySettings {
    /// Check the block. Called only while MCP is enabled.
    pub fn validate(&self) -> Result<(), McpIdentityError> {
        match self {
            Self::Fixed { realm } if is_valid_realm(realm) => Ok(()),
            Self::Fixed { .. } => Err(McpIdentityError::Realm),
        }
    }
}

/// Whether `realm` is a realm name MCP accepts: 1 to
/// [`MCP_REALM_MAX_CHARS`] ASCII letters, digits, `.`, `_`, or `-`.
///
/// - valid: `mcp`, `browser-demo`, `demo_sso.v2`
/// - invalid: `` (empty), `demo sso`, `realm/x`, `réalm`, 65 letters
pub fn is_valid_realm(realm: &str) -> bool {
    (1..=MCP_REALM_MAX_CHARS).contains(&realm.len())
        && realm
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(value: serde_json::Value) -> Result<McpIdentitySettings, serde_json::Error> {
        serde_json::from_value(value)
    }

    #[test]
    fn default_is_the_fixed_mcp_realm() {
        let expected = McpIdentitySettings::Fixed {
            realm: "mcp".to_string(),
        };
        assert_eq!(McpIdentitySettings::default(), expected);
        assert_eq!(parse(json!({"source": "fixed"})).unwrap(), expected);
        assert_eq!(
            parse(json!({"source": "fixed", "realm": "demo"})).unwrap(),
            McpIdentitySettings::Fixed {
                realm: "demo".to_string()
            }
        );
    }

    #[test]
    fn source_is_required_and_known() {
        for value in [
            json!({"realm": "mcp"}),
            json!({"source": "claims"}),
            json!({"source": "Fixed"}),
            json!({"source": null}),
        ] {
            assert!(parse(value.clone()).is_err(), "{value}");
        }
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let value = json!({"source": "fixed", "realm_claim": "realm"});
        assert!(parse(value).is_err());
    }

    #[test]
    fn fixed_realm_must_be_a_safe_name() {
        for realm in ["mcp", "browser-demo", "demo_sso.v2", &"r".repeat(64)] {
            let settings = McpIdentitySettings::Fixed {
                realm: realm.to_string(),
            };
            assert_eq!(settings.validate(), Ok(()), "{realm}");
        }
        for realm in ["", " mcp", "demo sso", "realm/x", "réalm", &"r".repeat(65)] {
            let settings = McpIdentitySettings::Fixed {
                realm: realm.to_string(),
            };
            assert_eq!(settings.validate(), Err(McpIdentityError::Realm), "{realm}");
        }
    }
}
