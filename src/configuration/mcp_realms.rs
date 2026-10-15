// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Checks of the MCP identity against the realms the REST configuration
//! uses. They need `auth.admin_roles` and the stream schemas, so they run
//! at startup next to [`McpSettings::validate`].

use crate::configuration::{
    EventSchema, McpConfigError, McpIdentitySettings, McpSettings, is_valid_realm, is_valid_role,
};
use std::collections::HashMap;
use std::fmt;

/// Where a realm key appears in the REST configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum RealmUse<'a> {
    /// A key of `auth.admin_roles`.
    Admin,
    /// A key of the `read_roles` of this event type.
    Read(&'a str),
    /// A key of the `write_roles` of this event type.
    Write(&'a str),
}

impl fmt::Display for RealmUse<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Admin => write!(f, "auth.admin_roles"),
            Self::Read(event_type) => {
                write!(f, "notification_schema.{event_type}.auth.read_roles")
            }
            Self::Write(event_type) => {
                write!(f, "notification_schema.{event_type}.auth.write_roles")
            }
        }
    }
}

impl McpSettings {
    /// Check the MCP identity against the REST role maps:
    /// `auth.admin_roles` and every stream's `read_roles` and
    /// `write_roles`. A disabled block always passes.
    ///
    /// With the fixed source, the MCP realm must not key `auth.admin_roles`
    /// or any `write_roles`. Those realms belong to REST callers, since
    /// MCP is read-only and a fixed realm never grants admin; sharing one
    /// would let token roles match rules written for those callers.
    /// `read_roles` may name it; that is how streams are opened to MCP
    /// users, and every stream that names it is readable by MCP tokens.
    ///
    /// With token claims, every realm key and role of the maps MCP users
    /// are matched against must be one a token can carry: `read_roles`
    /// always, and `auth.admin_roles` when the identity grants admin. A
    /// rule written with anything else would hold for REST users and
    /// silently never for MCP users. `write_roles` are not checked, since
    /// MCP never writes.
    pub fn validate_rest_realms(
        &self,
        admin_roles: &HashMap<String, Vec<String>>,
        schemas: Option<&HashMap<String, EventSchema>>,
    ) -> Result<(), McpConfigError> {
        if !self.enabled {
            return Ok(());
        }
        let realms = rest_realms(admin_roles, schemas);
        match &self.identity {
            McpIdentitySettings::Fixed { realm } => realms
                .iter()
                .find(|entry| entry.realm == realm && !matches!(entry.place, RealmUse::Read(_)))
                .map_or(Ok(()), |entry| {
                    Err(McpConfigError::RealmInUse {
                        realm: realm.clone(),
                        location: entry.place.to_string(),
                    })
                }),
            McpIdentitySettings::TokenClaims { .. } => {
                let admin = self.identity.grants_admin();
                let matched: Vec<&RealmEntry<'_>> = realms
                    .iter()
                    .filter(|entry| match entry.place {
                        RealmUse::Read(_) => true,
                        RealmUse::Admin => admin,
                        RealmUse::Write(_) => false,
                    })
                    .collect();
                if let Some(entry) = matched.iter().find(|entry| !is_valid_realm(entry.realm)) {
                    return Err(McpConfigError::RealmNotCarried {
                        realm: entry.realm.to_string(),
                        location: entry.place.to_string(),
                    });
                }
                matched
                    .iter()
                    .flat_map(|entry| entry.roles.iter().map(move |role| (entry, role)))
                    .find(|(_, role)| !is_valid_role(role))
                    .map_or(Ok(()), |(entry, role)| {
                        Err(McpConfigError::RoleNotCarried {
                            role: role.clone(),
                            location: format!("{}.{}", entry.place, entry.realm),
                        })
                    })
            }
        }
    }
}

/// One realm key of a REST role map, with its roles and where it is.
struct RealmEntry<'a> {
    realm: &'a str,
    place: RealmUse<'a>,
    roles: &'a [String],
}

/// Every realm key of the REST role maps, sorted so the first problem
/// reported does not depend on map order.
fn rest_realms<'a>(
    admin_roles: &'a HashMap<String, Vec<String>>,
    schemas: Option<&'a HashMap<String, EventSchema>>,
) -> Vec<RealmEntry<'a>> {
    let entries = |roles: &'a HashMap<String, Vec<String>>, place: RealmUse<'a>| {
        roles.iter().map(move |(realm, roles)| RealmEntry {
            realm,
            place,
            roles,
        })
    };
    let mut realms: Vec<RealmEntry<'a>> = entries(admin_roles, RealmUse::Admin).collect();
    for (event_type, schema) in schemas.into_iter().flatten() {
        let Some(auth) = &schema.auth else {
            continue;
        };
        if let Some(roles) = &auth.read_roles {
            realms.extend(entries(roles, RealmUse::Read(event_type)));
        }
        if let Some(roles) = &auth.write_roles {
            realms.extend(entries(roles, RealmUse::Write(event_type)));
        }
    }
    realms.sort_by(|a, b| (a.realm, a.place).cmp(&(b.realm, b.place)));
    realms
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::MCP_REALM_MAX_CHARS;
    use serde_json::json;

    fn enabled() -> McpSettings {
        serde_json::from_value(json!({
            "enabled": true,
            "resource_url": "https://aviso.example.int/mcp",
            "oauth": {
                "issuer": "https://auth.example.int/",
                "jwks_uri": "https://auth.example.int/.well-known/jwks.json"
            }
        }))
        .unwrap()
    }

    fn token_claims() -> McpSettings {
        let mut settings = enabled();
        settings.identity = serde_json::from_value(json!({"source": "token_claims"})).unwrap();
        settings
    }

    fn token_claims_with_admin() -> McpSettings {
        let mut settings = enabled();
        settings.identity =
            serde_json::from_value(json!({"source": "token_claims", "admin": true})).unwrap();
        settings
    }

    fn schemas(auth: serde_json::Value) -> HashMap<String, EventSchema> {
        let schema = json!({"identifier": {}, "auth": auth});
        HashMap::from([("ops".to_string(), serde_json::from_value(schema).unwrap())])
    }

    fn admins(realm: &str) -> HashMap<String, Vec<String>> {
        HashMap::from([(realm.to_string(), vec!["admin".to_string()])])
    }

    #[test]
    fn realm_must_not_be_a_rest_realm() {
        let settings = enabled();
        assert!(matches!(
            settings.validate_rest_realms(&admins("mcp"), None),
            Err(McpConfigError::RealmInUse { .. })
        ));
        let writers = schemas(json!({"required": true, "write_roles": {"mcp": ["w"]}}));
        let error = settings
            .validate_rest_realms(&HashMap::new(), Some(&writers))
            .unwrap_err();
        assert!(
            error.to_string().contains("ops.auth.write_roles"),
            "{error}"
        );
        let readers = schemas(json!({"required": true, "read_roles": {"mcp": ["r"]}}));
        assert_eq!(
            settings.validate_rest_realms(&HashMap::new(), Some(&readers)),
            Ok(())
        );
        let mut disabled = enabled();
        disabled.enabled = false;
        assert_eq!(disabled.validate_rest_realms(&admins("mcp"), None), Ok(()));
    }

    #[test]
    fn token_claim_realms_may_be_rest_realms() {
        let writers = schemas(json!({"required": true, "write_roles": {"ecmwf": ["w"]}}));
        assert_eq!(
            token_claims().validate_rest_realms(&admins("ecmwf"), Some(&writers)),
            Ok(())
        );
    }

    #[test]
    fn token_claims_refuse_read_realms_no_token_can_carry() {
        let settings = token_claims();
        for realm in ["réalm", "a/b", "ECMWF staff"] {
            let auth = json!({"required": true, "read_roles": {"ecmwf": ["r"], realm: ["r"]}});
            let error = settings
                .validate_rest_realms(&HashMap::new(), Some(&schemas(auth)))
                .unwrap_err();
            assert_eq!(
                error,
                McpConfigError::RealmNotCarried {
                    realm: realm.to_string(),
                    location: "notification_schema.ops.auth.read_roles".to_string()
                }
            );
        }
        let long = "r".repeat(MCP_REALM_MAX_CHARS + 1);
        let readers = schemas(json!({"required": true, "read_roles": {long: ["r"]}}));
        assert!(matches!(
            settings.validate_rest_realms(&HashMap::new(), Some(&readers)),
            Err(McpConfigError::RealmNotCarried { .. })
        ));
    }

    fn role_error(role: &str, location: &str) -> McpConfigError {
        McpConfigError::RoleNotCarried {
            role: role.to_string(),
            location: location.to_string(),
        }
    }

    #[test]
    fn token_claims_refuse_read_roles_no_token_can_carry() {
        let long = "r".repeat(129);
        for role in ["bell\u{7}", " padded", long.as_str()] {
            let auth = json!({"required": true, "read_roles": {"ecmwf": ["reader", role]}});
            let error = token_claims()
                .validate_rest_realms(&HashMap::new(), Some(&schemas(auth)))
                .unwrap_err();
            let location = "notification_schema.ops.auth.read_roles.ecmwf";
            assert_eq!(error, role_error(role, location));
        }
        let fine = json!({"required": true, "read_roles": {"ecmwf": ["*", "data team"]}});
        assert_eq!(
            token_claims().validate_rest_realms(&HashMap::new(), Some(&schemas(fine))),
            Ok(())
        );
    }

    #[test]
    fn token_claims_ignore_write_roles() {
        let long = "r".repeat(129);
        let writers = schemas(json!({"required": true, "write_roles": {
            "ECMWF staff": ["w"],
            "ecmwf": [long]
        }}));
        for settings in [token_claims(), token_claims_with_admin()] {
            assert_eq!(
                settings.validate_rest_realms(&HashMap::new(), Some(&writers)),
                Ok(())
            );
        }
    }

    #[test]
    fn admin_roles_are_checked_only_when_the_identity_grants_admin() {
        let bad_realm = admins("ECMWF staff");
        let padded = HashMap::from([("ecmwf".to_string(), vec![" admin".to_string()])]);
        for map in [&bad_realm, &padded] {
            assert_eq!(token_claims().validate_rest_realms(map, None), Ok(()));
        }
        assert_eq!(
            token_claims_with_admin().validate_rest_realms(&bad_realm, None),
            Err(McpConfigError::RealmNotCarried {
                realm: "ECMWF staff".to_string(),
                location: "auth.admin_roles".to_string()
            })
        );
        assert_eq!(
            token_claims_with_admin().validate_rest_realms(&padded, None),
            Err(role_error(" admin", "auth.admin_roles.ecmwf"))
        );
    }

    #[test]
    fn the_first_problem_is_reported_in_a_stable_order() {
        let admins = HashMap::from([
            ("z z".to_string(), vec!["admin".to_string()]),
            ("a a".to_string(), vec!["admin".to_string()]),
        ]);
        for _ in 0..8 {
            let error = token_claims_with_admin()
                .validate_rest_realms(&admins, None)
                .unwrap_err();
            assert!(error.to_string().contains("'a a'"), "{error}");
        }
    }

    #[test]
    fn fixed_source_does_not_check_roles() {
        let long = "r".repeat(129);
        let readers = schemas(json!({"required": true, "read_roles": {"mcp": [long]}}));
        assert_eq!(
            enabled().validate_rest_realms(&HashMap::new(), Some(&readers)),
            Ok(())
        );
    }
}
