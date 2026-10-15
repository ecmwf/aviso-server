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
//!
//! ```yaml
//! identity:
//!   source: token_claims   # realm and roles from the access token
//!   realm_claim: "realm"
//!   roles_claim: "roles"
//! ```

use serde::de::{self, Unexpected, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;
use thiserror::Error;

/// Longest realm accepted, in characters, from configuration or a token.
pub const MCP_REALM_MAX_CHARS: usize = 64;

/// Longest identity claim name accepted, in characters.
const MAX_CLAIM_NAME_CHARS: usize = 128;

/// Claims the token verifier already reads for its own checks. An
/// identity claim must not reuse one of them.
const VERIFIER_CLAIMS: [&str; 9] = [
    "iss",
    "sub",
    "aud",
    "exp",
    "nbf",
    "iat",
    "jti",
    "client_id",
    "scope",
];

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
    /// The realm and roles come from claims the issuer adds to the token.
    /// A token without both claims is refused. Claim names are literal
    /// top-level keys of the token payload: `realm_access.roles` names a
    /// key with a dot in it, not the `roles` field inside `realm_access`.
    TokenClaims {
        /// Claim holding the realm, a string.
        #[serde(default = "default_realm_claim")]
        realm_claim: String,
        /// Claim holding the roles, an array of strings.
        #[serde(default = "default_roles_claim")]
        roles_claim: String,
        /// Match token realms and roles against the REST
        /// `auth.admin_roles`, so a REST admin is an admin over MCP too.
        /// Off by default: turning it on lets the issuer of MCP tokens
        /// grant admin rights.
        #[serde(default, deserialize_with = "strict_bool")]
        admin: bool,
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

fn default_realm_claim() -> String {
    "realm".to_string()
}

fn default_roles_claim() -> String {
    "roles".to_string()
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
    /// A claim name is empty, too long, or has whitespace or control
    /// characters.
    #[error(
        "mcp.identity.{field} must be 1 to {MAX_CLAIM_NAME_CHARS} characters \
         without whitespace or control characters"
    )]
    ClaimName { field: &'static str },
    /// A claim name is one the verifier already reads.
    #[error(
        "mcp.identity.{field} '{claim}' is checked by the token verifier \
         and cannot carry identity"
    )]
    VerifierClaim { field: &'static str, claim: String },
    /// Both claim names are the same.
    #[error("mcp.identity.realm_claim and mcp.identity.roles_claim must differ")]
    SameClaim,
}

/// Read a security flag strictly. Environment variables reach the
/// fields of this internally tagged enum as text, so the strings `true`
/// and `false` are accepted too, and nothing else.
///
/// - valid: `true`, `false`, `"true"`, `"false"`
/// - invalid: `"yes"`, `"True"`, `"1"`, `1`, `null`
fn strict_bool<'de, D: Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    struct StrictBool;

    impl Visitor<'_> for StrictBool {
        type Value = bool;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("true or false")
        }

        fn visit_bool<E: de::Error>(self, value: bool) -> Result<bool, E> {
            Ok(value)
        }

        fn visit_str<E: de::Error>(self, value: &str) -> Result<bool, E> {
            match value {
                "true" => Ok(true),
                "false" => Ok(false),
                other => Err(E::invalid_value(Unexpected::Str(other), &self)),
            }
        }
    }

    deserializer.deserialize_any(StrictBool)
}

impl McpIdentitySettings {
    /// Whether the REST `auth.admin_roles` apply to MCP users. Only token
    /// claim identities with `admin: true` opt in.
    pub fn grants_admin(&self) -> bool {
        matches!(self, Self::TokenClaims { admin: true, .. })
    }

    /// Check the block. Called only while MCP is enabled.
    pub fn validate(&self) -> Result<(), McpIdentityError> {
        match self {
            Self::Fixed { realm } if is_valid_realm(realm) => Ok(()),
            Self::Fixed { .. } => Err(McpIdentityError::Realm),
            Self::TokenClaims {
                realm_claim,
                roles_claim,
                ..
            } => {
                check_claim_name("realm_claim", realm_claim)?;
                check_claim_name("roles_claim", roles_claim)?;
                if realm_claim == roles_claim {
                    return Err(McpIdentityError::SameClaim);
                }
                Ok(())
            }
        }
    }
}

/// Whether `realm` is a realm name MCP accepts, from configuration or
/// from a token: 1 to [`MCP_REALM_MAX_CHARS`] ASCII letters, digits,
/// `.`, `_`, or `-`.
///
/// - valid: `mcp`, `browser-demo`, `demo_sso.v2`
/// - invalid: `` (empty), `demo sso`, `realm/x`, `réalm`, 65 letters
pub fn is_valid_realm(realm: &str) -> bool {
    (1..=MCP_REALM_MAX_CHARS).contains(&realm.len())
        && realm
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

/// Accept a JSON object key usable as an identity claim.
///
/// - valid: `realm`, `aviso_roles`, `https://aviso.example.int/roles`
/// - invalid: `` (empty), `aviso roles`, `sub`
fn check_claim_name(field: &'static str, name: &str) -> Result<(), McpIdentityError> {
    let well_formed = !name.is_empty()
        && name.chars().count() <= MAX_CLAIM_NAME_CHARS
        && !name.chars().any(|c| c.is_whitespace() || c.is_control());
    if !well_formed {
        return Err(McpIdentityError::ClaimName { field });
    }
    if VERIFIER_CLAIMS.contains(&name) {
        return Err(McpIdentityError::VerifierClaim {
            field,
            claim: name.to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(value: serde_json::Value) -> Result<McpIdentitySettings, serde_json::Error> {
        serde_json::from_value(value)
    }

    fn token_claims(realm_claim: &str, roles_claim: &str) -> McpIdentitySettings {
        McpIdentitySettings::TokenClaims {
            realm_claim: realm_claim.to_string(),
            roles_claim: roles_claim.to_string(),
            admin: false,
        }
    }

    #[test]
    fn default_is_the_fixed_mcp_realm() {
        let expected = McpIdentitySettings::Fixed {
            realm: "mcp".to_string(),
        };
        assert_eq!(McpIdentitySettings::default(), expected);
        assert_eq!(parse(json!({"source": "fixed"})).unwrap(), expected);
    }

    #[test]
    fn both_sources_parse() {
        assert_eq!(
            parse(json!({"source": "fixed", "realm": "demo"})).unwrap(),
            McpIdentitySettings::Fixed {
                realm: "demo".to_string()
            }
        );
        assert_eq!(
            parse(json!({"source": "token_claims"})).unwrap(),
            token_claims("realm", "roles")
        );
        let custom = json!({
            "source": "token_claims",
            "realm_claim": "aviso_realm",
            "roles_claim": "aviso_roles"
        });
        assert_eq!(
            parse(custom).unwrap(),
            token_claims("aviso_realm", "aviso_roles")
        );
    }

    #[test]
    fn admin_rights_are_an_explicit_opt_in() {
        let default = parse(json!({"source": "token_claims"})).unwrap();
        assert!(!default.grants_admin());
        assert!(!McpIdentitySettings::default().grants_admin());
        for (admin, grants) in [
            (json!(true), true),
            (json!("true"), true),
            (json!(false), false),
            (json!("false"), false),
        ] {
            let settings = parse(json!({"source": "token_claims", "admin": admin})).unwrap();
            assert_eq!(settings.grants_admin(), grants, "{admin}");
        }
        for admin in [
            json!("yes"),
            json!("True"),
            json!("1"),
            json!(1),
            json!(null),
        ] {
            let value = json!({"source": "token_claims", "admin": admin});
            assert!(parse(value).is_err(), "{admin}");
        }
        assert!(parse(json!({"source": "fixed", "admin": true})).is_err());
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
    fn fields_of_the_other_source_are_rejected() {
        for value in [
            json!({"source": "fixed", "realm_claim": "realm"}),
            json!({"source": "token_claims", "realm": "mcp"}),
            json!({"source": "token_claims", "role_claim": "roles"}),
        ] {
            assert!(parse(value.clone()).is_err(), "{value}");
        }
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

    #[test]
    fn claim_names_are_checked() {
        assert_eq!(token_claims("realm", "roles").validate(), Ok(()));
        assert_eq!(
            token_claims("https://aviso.example.int/realm", "aviso_roles").validate(),
            Ok(())
        );
        for (realm_claim, roles_claim, field) in [
            ("", "roles", "realm_claim"),
            ("realm", "aviso roles", "roles_claim"),
            ("realm", "roles\n", "roles_claim"),
            (&"r".repeat(129), "roles", "realm_claim"),
        ] {
            assert_eq!(
                token_claims(realm_claim, roles_claim).validate(),
                Err(McpIdentityError::ClaimName { field }),
                "{realm_claim:?} {roles_claim:?}"
            );
        }
        assert_eq!(
            token_claims("roles", "roles").validate(),
            Err(McpIdentityError::SameClaim)
        );
    }

    #[test]
    fn verifier_claims_cannot_carry_identity() {
        for claim in VERIFIER_CLAIMS {
            let error = token_claims(claim, "roles").validate().unwrap_err();
            assert_eq!(
                error,
                McpIdentityError::VerifierClaim {
                    field: "realm_claim",
                    claim: claim.to_string()
                }
            );
        }
        assert!(matches!(
            token_claims("realm", "scope").validate(),
            Err(McpIdentityError::VerifierClaim {
                field: "roles_claim",
                ..
            })
        ));
    }
}
