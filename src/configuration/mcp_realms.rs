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

use crate::configuration::{EventSchema, McpConfigError, McpIdentitySettings, McpSettings};
use std::collections::HashMap;

impl McpSettings {
    /// Check that users of the fixed MCP realm cannot pass for REST
    /// users. A realm that keys `auth.admin_roles` or any stream's
    /// `write_roles` belongs to REST callers, since MCP is read-only and
    /// a fixed realm never grants admin. Sharing it would let token roles
    /// match rules written for those callers. Stream `read_roles` may
    /// name the realm; that is how streams are opened to MCP users, and
    /// every stream that names it is readable by MCP tokens.
    ///
    /// Realms from token claims are the issuer's own realms, shared with
    /// REST on purpose, so they are not checked. A disabled block always
    /// passes.
    pub fn validate_realm_isolation(
        &self,
        admin_roles: &HashMap<String, Vec<String>>,
        schemas: Option<&HashMap<String, EventSchema>>,
    ) -> Result<(), McpConfigError> {
        if !self.enabled {
            return Ok(());
        }
        let McpIdentitySettings::Fixed { realm } = &self.identity else {
            return Ok(());
        };
        let in_use = |location: String| McpConfigError::RealmInUse {
            realm: realm.clone(),
            location,
        };
        if admin_roles.contains_key(realm) {
            return Err(in_use("auth.admin_roles".to_string()));
        }
        let mut event_types: Vec<&String> = schemas
            .into_iter()
            .flatten()
            .filter(|(_, schema)| {
                schema
                    .auth
                    .as_ref()
                    .and_then(|auth| auth.write_roles.as_ref())
                    .is_some_and(|roles| roles.contains_key(realm))
            })
            .map(|(event_type, _)| event_type)
            .collect();
        event_types.sort();
        match event_types.first() {
            Some(event_type) => Err(in_use(format!(
                "notification_schema.{event_type}.auth.write_roles"
            ))),
            None => Ok(()),
        }
    }
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

    fn schemas(auth: serde_json::Value) -> HashMap<String, EventSchema> {
        let schema = json!({"identifier": {}, "auth": auth});
        HashMap::from([("ops".to_string(), serde_json::from_value(schema).unwrap())])
    }

    #[test]
    fn realm_must_not_be_a_rest_realm() {
        let settings = enabled();
        let admins = HashMap::from([("mcp".to_string(), vec!["admin".to_string()])]);
        assert!(matches!(
            settings.validate_realm_isolation(&admins, None),
            Err(McpConfigError::RealmInUse { .. })
        ));
        let writers = schemas(json!({"required": true, "write_roles": {"mcp": ["w"]}}));
        let error = settings
            .validate_realm_isolation(&HashMap::new(), Some(&writers))
            .unwrap_err();
        assert!(
            error.to_string().contains("ops.auth.write_roles"),
            "{error}"
        );
        let readers = schemas(json!({"required": true, "read_roles": {"mcp": ["r"]}}));
        assert_eq!(
            settings.validate_realm_isolation(&HashMap::new(), Some(&readers)),
            Ok(())
        );
        let mut disabled = enabled();
        disabled.enabled = false;
        assert_eq!(disabled.validate_realm_isolation(&admins, None), Ok(()));
    }

    #[test]
    fn token_claim_realms_may_be_rest_realms() {
        let mut settings = enabled();
        settings.identity = serde_json::from_value(json!({"source": "token_claims"})).unwrap();
        let admins = HashMap::from([("ecmwf".to_string(), vec!["admin".to_string()])]);
        let writers = schemas(json!({"required": true, "write_roles": {"ecmwf": ["w"]}}));
        assert_eq!(
            settings.validate_realm_isolation(&admins, Some(&writers)),
            Ok(())
        );
    }
}
