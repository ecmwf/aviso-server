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
    EventSchema, McpConfigError, McpIdentitySettings, McpSettings, is_valid_realm,
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
    /// Check the MCP identity against every realm key of
    /// `auth.admin_roles` and of the stream `read_roles` and
    /// `write_roles`. A disabled block always passes.
    ///
    /// With the fixed source, the MCP realm must not key `auth.admin_roles`
    /// or any `write_roles`. Those realms belong to REST callers, since
    /// MCP is read-only and a fixed realm never grants admin; sharing one
    /// would let token roles match rules written for those callers.
    /// `read_roles` may name it; that is how streams are opened to MCP
    /// users, and every stream that names it is readable by MCP tokens.
    ///
    /// With token claims, every REST realm must be a realm a token can
    /// carry. A rule keyed by any other realm would hold for REST users
    /// and silently never for MCP users.
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
                .find(|(key, place)| key == realm && !matches!(place, RealmUse::Read(_)))
                .map_or(Ok(()), |(_, place)| {
                    Err(McpConfigError::RealmInUse {
                        realm: realm.clone(),
                        location: place.to_string(),
                    })
                }),
            McpIdentitySettings::TokenClaims { .. } => realms
                .iter()
                .find(|(key, _)| !is_valid_realm(key))
                .map_or(Ok(()), |(key, place)| {
                    Err(McpConfigError::RealmNotCarried {
                        realm: (*key).to_string(),
                        location: place.to_string(),
                    })
                }),
        }
    }
}

/// Every realm key of the REST role maps with where it appears, sorted so
/// the first problem reported does not depend on map order.
fn rest_realms<'a>(
    admin_roles: &'a HashMap<String, Vec<String>>,
    schemas: Option<&'a HashMap<String, EventSchema>>,
) -> Vec<(&'a str, RealmUse<'a>)> {
    let mut realms: Vec<(&str, RealmUse<'_>)> = admin_roles
        .keys()
        .map(|realm| (realm.as_str(), RealmUse::Admin))
        .collect();
    for (event_type, schema) in schemas.into_iter().flatten() {
        let Some(auth) = &schema.auth else {
            continue;
        };
        let maps = [
            (&auth.read_roles, RealmUse::Read(event_type.as_str())),
            (&auth.write_roles, RealmUse::Write(event_type.as_str())),
        ];
        for (roles, place) in maps {
            realms.extend(
                roles
                    .iter()
                    .flat_map(HashMap::keys)
                    .map(|realm| (realm.as_str(), place)),
            );
        }
    }
    realms.sort();
    realms
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn token_claims_refuse_rest_realms_no_token_can_carry() {
        let settings = token_claims();
        let error = settings
            .validate_rest_realms(&admins("ECMWF staff"), None)
            .unwrap_err();
        assert_eq!(
            error,
            McpConfigError::RealmNotCarried {
                realm: "ECMWF staff".to_string(),
                location: "auth.admin_roles".to_string()
            }
        );
        for (field, realm) in [("read_roles", "réalm"), ("write_roles", "a/b")] {
            let auth = json!({"required": true, field: {"ecmwf": ["r"], realm: ["r"]}});
            let error = settings
                .validate_rest_realms(&admins("ecmwf"), Some(&schemas(auth)))
                .unwrap_err();
            let message = error.to_string();
            assert!(message.contains(&format!("'{realm}'")), "{message}");
            assert!(
                message.contains(&format!("notification_schema.ops.auth.{field}")),
                "{message}"
            );
        }
        let long = "r".repeat(65);
        let readers = schemas(json!({"required": true, "read_roles": {long: ["r"]}}));
        assert!(matches!(
            settings.validate_rest_realms(&HashMap::new(), Some(&readers)),
            Err(McpConfigError::RealmNotCarried { .. })
        ));
    }

    #[test]
    fn the_first_problem_is_reported_in_a_stable_order() {
        let admins = HashMap::from([
            ("z z".to_string(), vec!["admin".to_string()]),
            ("a a".to_string(), vec!["admin".to_string()]),
        ]);
        for _ in 0..8 {
            let error = token_claims()
                .validate_rest_realms(&admins, None)
                .unwrap_err();
            assert!(error.to_string().contains("'a a'"), "{error}");
        }
    }
}
