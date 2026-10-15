// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Realm and roles of an MCP user, read from the claims of a verified
//! token.
//!
//! [`McpIdentitySettings`] picks the source. `fixed` gives every user the
//! configured realm and reads roles from the optional `roles` claim.
//! `token_claims` reads both from the configured claims and refuses a
//! token that lacks either one; there is no fallback realm.
//!
//! Claim names are literal top-level keys of the token payload. A claim
//! whose value is `null` counts as absent.

use crate::configuration::{MCP_REALM_MAX_CHARS, McpIdentitySettings, is_valid_realm};
use serde_json::{Map, Value};
use thiserror::Error;

/// Most roles taken from one token. Tokens are capped at 16 KiB, so this
/// mainly guards against a claim padded with tiny entries.
pub const MAX_ROLES: usize = 256;
/// Longest role accepted, in characters.
pub const MAX_ROLE_CHARS: usize = 128;
/// Claim that carries the roles under the `fixed` source.
const FIXED_ROLES_CLAIM: &str = "roles";

/// Why the identity claims of a token were refused.
///
/// Messages describe the rule that failed and never echo claim values,
/// so they are safe to send back to the client.
#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum IdentityClaimError {
    /// Neither the realm nor the roles claim is present, typically
    /// because the issuer does not add them for this resource.
    #[error("realm and roles claims are missing")]
    MissingClaims,
    /// The roles claim is present without the realm claim.
    #[error("realm claim is missing")]
    MissingRealm,
    /// The realm claim is present without the roles claim.
    #[error("roles claim is missing")]
    MissingRoles,
    /// The realm claim is not a string.
    #[error("realm claim is not a string")]
    RealmType,
    /// The realm is empty, too long, or uses other characters.
    #[error(
        "realm claim must be 1 to {MCP_REALM_MAX_CHARS} characters of \
         A-Z, a-z, 0-9, '.', '_' or '-'"
    )]
    RealmFormat,
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

/// Realm and roles of one verified token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenIdentity {
    /// Realm matched against the keys of stream role maps.
    pub realm: String,
    /// Roles matched against the realm's role list.
    pub roles: Vec<String>,
}

/// Resolve the identity of a token from its claims under `settings`.
pub fn resolve_identity(
    settings: &McpIdentitySettings,
    claims: &Map<String, Value>,
) -> Result<TokenIdentity, IdentityClaimError> {
    match settings {
        McpIdentitySettings::Fixed { realm } => {
            let roles = claim(claims, FIXED_ROLES_CLAIM)
                .map(parse_roles)
                .transpose()?
                .unwrap_or_default();
            Ok(TokenIdentity {
                realm: realm.clone(),
                roles,
            })
        }
        McpIdentitySettings::TokenClaims {
            realm_claim,
            roles_claim,
        } => match (claim(claims, realm_claim), claim(claims, roles_claim)) {
            (None, None) => Err(IdentityClaimError::MissingClaims),
            (None, Some(_)) => Err(IdentityClaimError::MissingRealm),
            (Some(_), None) => Err(IdentityClaimError::MissingRoles),
            (Some(realm), Some(roles)) => Ok(TokenIdentity {
                realm: parse_realm(realm)?,
                roles: parse_roles(roles)?,
            }),
        },
    }
}

/// The value of the top-level claim `name`, unless it is absent or null.
///
/// - `{"realm_access": {"roles": []}}` has no claim `realm_access.roles`
/// - `{"realm": null}` has no claim `realm`
fn claim<'a>(claims: &'a Map<String, Value>, name: &str) -> Option<&'a Value> {
    claims.get(name).filter(|value| !value.is_null())
}

/// Parse a realm claim value.
///
/// - valid: `"browser-demo"`, `"ecmwf"`
/// - invalid: `""`, `"demo sso"`, `7`, `["ecmwf"]`
fn parse_realm(value: &Value) -> Result<String, IdentityClaimError> {
    let realm = value.as_str().ok_or(IdentityClaimError::RealmType)?;
    if is_valid_realm(realm) {
        Ok(realm.to_string())
    } else {
        Err(IdentityClaimError::RealmFormat)
    }
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

    fn fixed() -> McpIdentitySettings {
        McpIdentitySettings::Fixed {
            realm: "mcp".to_string(),
        }
    }

    fn token_claims() -> McpIdentitySettings {
        McpIdentitySettings::TokenClaims {
            realm_claim: "realm".to_string(),
            roles_claim: "roles".to_string(),
        }
    }

    fn resolve(
        settings: &McpIdentitySettings,
        claims: Value,
    ) -> Result<TokenIdentity, IdentityClaimError> {
        match claims {
            Value::Object(map) => resolve_identity(settings, &map),
            other => panic!("claims must be an object, got {other}"),
        }
    }

    fn identity(realm: &str, roles: &[&str]) -> TokenIdentity {
        TokenIdentity {
            realm: realm.to_string(),
            roles: roles.iter().map(|role| role.to_string()).collect(),
        }
    }

    #[test]
    fn fixed_source_uses_the_configured_realm_and_optional_roles() {
        assert_eq!(resolve(&fixed(), json!({})), Ok(identity("mcp", &[])));
        let null = json!({"roles": null});
        assert_eq!(resolve(&fixed(), null), Ok(identity("mcp", &[])));
        let claims = json!({"realm": "ecmwf", "roles": ["aviso-mcp", "reader"]});
        assert_eq!(
            resolve(&fixed(), claims),
            Ok(identity("mcp", &["aviso-mcp", "reader"]))
        );
    }

    #[test]
    fn token_claims_source_reads_both_claims() {
        let claims = json!({"realm": "browser-demo", "roles": ["reader"]});
        assert_eq!(
            resolve(&token_claims(), claims),
            Ok(identity("browser-demo", &["reader"]))
        );
        let empty = json!({"realm": "ecmwf", "roles": []});
        assert_eq!(resolve(&token_claims(), empty), Ok(identity("ecmwf", &[])));
    }

    #[test]
    fn configured_claim_names_are_used() {
        let settings = McpIdentitySettings::TokenClaims {
            realm_claim: "aviso_realm".to_string(),
            roles_claim: "aviso_roles".to_string(),
        };
        let claims = json!({
            "realm": "other", "roles": ["other"],
            "aviso_realm": "demo-sso", "aviso_roles": ["reader"]
        });
        assert_eq!(
            resolve(&settings, claims),
            Ok(identity("demo-sso", &["reader"]))
        );
        let only_defaults = json!({"realm": "ecmwf", "roles": ["reader"]});
        assert_eq!(
            resolve(&settings, only_defaults),
            Err(IdentityClaimError::MissingClaims)
        );
    }

    #[test]
    fn claim_names_are_top_level_keys_not_paths() {
        let settings = McpIdentitySettings::TokenClaims {
            realm_claim: "realm".to_string(),
            roles_claim: "realm_access.roles".to_string(),
        };
        let nested = json!({"realm": "ecmwf", "realm_access": {"roles": ["reader"]}});
        assert_eq!(
            resolve(&settings, nested),
            Err(IdentityClaimError::MissingRoles)
        );
        let literal = json!({"realm": "ecmwf", "realm_access.roles": ["reader"]});
        assert_eq!(
            resolve(&settings, literal),
            Ok(identity("ecmwf", &["reader"]))
        );
    }

    #[test]
    fn token_claims_source_has_no_fallback() {
        for (claims, error) in [
            (json!({}), IdentityClaimError::MissingClaims),
            (
                json!({"realm": null, "roles": null}),
                IdentityClaimError::MissingClaims,
            ),
            (
                json!({"roles": ["reader"]}),
                IdentityClaimError::MissingRealm,
            ),
            (
                json!({"realm": null, "roles": []}),
                IdentityClaimError::MissingRealm,
            ),
            (json!({"realm": "ecmwf"}), IdentityClaimError::MissingRoles),
            (
                json!({"realm": "ecmwf", "roles": null}),
                IdentityClaimError::MissingRoles,
            ),
        ] {
            assert_eq!(
                resolve(&token_claims(), claims.clone()),
                Err(error),
                "{claims}"
            );
        }
    }

    #[test]
    fn realm_of_the_wrong_type_or_shape_is_refused() {
        for realm in [
            json!(7),
            json!(true),
            json!(["ecmwf"]),
            json!({"name": "ecmwf"}),
        ] {
            let claims = json!({"realm": realm, "roles": []});
            assert_eq!(
                resolve(&token_claims(), claims),
                Err(IdentityClaimError::RealmType),
                "{realm}"
            );
        }
        let long = "r".repeat(MCP_REALM_MAX_CHARS + 1);
        for realm in [
            "",
            " ecmwf",
            "demo sso",
            "a/b",
            "r\u{e9}alm",
            "x\ny",
            long.as_str(),
        ] {
            let claims = json!({"realm": realm, "roles": []});
            assert_eq!(
                resolve(&token_claims(), claims),
                Err(IdentityClaimError::RealmFormat),
                "{realm:?}"
            );
        }
        let longest = "r".repeat(MCP_REALM_MAX_CHARS);
        let claims = json!({"realm": longest, "roles": []});
        assert!(resolve(&token_claims(), claims).is_ok());
    }

    #[test]
    fn roles_of_the_wrong_type_are_refused() {
        for roles in [json!("reader"), json!([1]), json!([null]), json!({"r": 1})] {
            for settings in [fixed(), token_claims()] {
                let claims = json!({"realm": "ecmwf", "roles": roles});
                assert_eq!(
                    resolve(&settings, claims),
                    Err(IdentityClaimError::RolesType),
                    "{roles}"
                );
            }
        }
    }

    #[test]
    fn roles_out_of_bounds_are_refused() {
        let claims = |roles: Value| json!({"realm": "ecmwf", "roles": roles});
        let many = json!(vec!["r"; MAX_ROLES + 1]);
        assert_eq!(
            resolve(&token_claims(), claims(many)),
            Err(IdentityClaimError::TooManyRoles)
        );
        let at_limit = json!(vec!["r"; MAX_ROLES]);
        assert!(resolve(&token_claims(), claims(at_limit)).is_ok());
        let long = "r".repeat(MAX_ROLE_CHARS + 1);
        for role in [
            "",
            " ",
            " reader",
            "reader\u{a0}",
            "role\nwith-newline",
            "tab\there",
            "nul\u{0}",
            long.as_str(),
        ] {
            for settings in [fixed(), token_claims()] {
                assert_eq!(
                    resolve(&settings, claims(json!([role]))),
                    Err(IdentityClaimError::RoleFormat),
                    "{role:?}"
                );
            }
        }
        let longest = "r".repeat(MAX_ROLE_CHARS);
        let inner_space = claims(json!([longest, "data team", "*"]));
        assert!(resolve(&token_claims(), inner_space).is_ok());
    }
}
