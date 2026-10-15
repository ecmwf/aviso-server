// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Per-user read access over `/mcp`, end to end.
//!
//! One server runs the REST API in trusted proxy mode and the MCP endpoint
//! side by side over the same schema. The same realm and roles reach REST
//! through an HS256 JWT and MCP through a signed access token, and the
//! read decisions of both are checked.

#[path = "common/mcp_issuer.rs"]
mod mcp_issuer;

use actix_web::dev::ServerHandle;
use aviso_server::configuration::{AuthMode, AuthSettings, Settings};
use aviso_server::mcp::auth::TokenVerifier;
use aviso_server::mcp::jwks::{JwksCache, JwksTiming};
use aviso_server::mcp::state::McpState;
use aviso_server::notification_backend::{NotificationBackend, build_backend};
use aviso_server::startup::{OptionalServices, run};
use jsonwebtoken::{EncodingKey, Header, encode};
use mcp_issuer::*;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

const REST_SECRET: &str = "mcp-token-roles-secret";

/// Streams whose REST read decision needs no auth plugin, so REST and MCP
/// decisions can be compared one to one.
const SHARED_STREAMS: [&str; 5] = ["open", "authed", "ecmwf_readers", "demo_realm", "mcp_fixed"];

/// What a user without matching roles or admin rights may read.
const ANYONE: &[&str] = &["authed", "open"];

/// Every shared stream, as a REST admin reads them.
const EVERY_SHARED: &[&str] = &["authed", "demo_realm", "ecmwf_readers", "mcp_fixed", "open"];

fn init_config() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        let stream = |base: &str, auth: Value| {
            let mut schema = json!({
                "topic": {"base": base, "key_order": ["class", "number"]},
                "identifier": {
                    "class": {"type": "StringHandler", "required": true},
                    "number": {"type": "IntHandler", "required": true}
                },
                "payload": {"required": false}
            });
            if !auth.is_null() {
                schema["auth"] = auth;
            }
            schema
        };
        let readers = |roles: Value| json!({"required": true, "read_roles": roles});
        let mut config: Settings = serde_json::from_value(json!({
            "application": {"host": "127.0.0.1", "port": 0},
            "notification_backend": {"kind": "in_memory"},
            "notification_schema": {
                "open": stream("RolesOpen", Value::Null),
                "authed": stream("RolesAuthed", json!({"required": true})),
                "ecmwf_readers": stream("RolesEcmwf", readers(json!({"ecmwf": ["reader"]}))),
                "demo_realm": stream("RolesDemo", readers(json!({"browser-demo": ["*"]}))),
                "mcp_fixed": stream("RolesFixed", readers(json!({"mcp": ["aviso-mcp"]}))),
                "plugged": stream("RolesPlugged",
                    json!({"required": true, "plugins": ["ecpds"]}))
            }
        }))
        .unwrap();
        config.watch_endpoint.replay_batch_delay_ms = 0;
        config.init_global_config();
    });
}

/// A realm to admin roles map with one `admin` role per realm.
fn admins(realms: &[&str]) -> HashMap<String, Vec<String>> {
    realms
        .iter()
        .map(|realm| (realm.to_string(), vec!["admin".to_string()]))
        .collect()
}

/// REST and MCP on one listener, plus the token issuer. Both share the
/// admin role map.
struct Server {
    address: String,
    jwks: JwksServer,
    handle: ServerHandle,
    task: tokio::task::JoinHandle<std::io::Result<()>>,
}

impl Server {
    async fn start(identity: Value, admin_roles: HashMap<String, Vec<String>>) -> Self {
        init_config();
        let backend: Arc<dyn NotificationBackend> =
            build_backend(&serde_json::from_value(json!({"kind": "in_memory"})).unwrap())
                .await
                .unwrap();
        for base in [
            "RolesOpen",
            "RolesAuthed",
            "RolesEcmwf",
            "RolesDemo",
            "RolesFixed",
        ] {
            let topic = format!("{base}.od.1");
            backend
                .put_messages(&topic, "{}".to_string())
                .await
                .unwrap();
        }
        let jwks = JwksServer::start(vec![jwk(&key_a(), KID_A)]);
        let mut settings = settings(&jwks.uri);
        settings.identity = serde_json::from_value(identity).unwrap();
        let timing = JwksTiming {
            min_refetch_interval: Duration::ZERO,
            ..JwksTiming::with_ttl(Duration::from_secs(300))
        };
        let verifier = TokenVerifier::new(JwksCache::new(&jwks.uri, timing).unwrap(), &settings);
        let state = McpState::new(&settings, verifier, &admin_roles).unwrap();
        let auth = AuthSettings {
            enabled: true,
            mode: AuthMode::TrustedProxy,
            jwt_secret: REST_SECRET.to_string(),
            admin_roles,
            ..AuthSettings::default()
        };
        let application = serde_json::from_value(json!({"host": "127.0.0.1", "port": 0})).unwrap();
        let mut optional = OptionalServices::default();
        optional.mcp = Some(actix_web::web::Data::new(state));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let server = run(
            listener,
            backend,
            CancellationToken::new(),
            Arc::new(auth),
            None,
            application,
            optional,
        )
        .unwrap();
        Self {
            address,
            jwks,
            handle: server.handle(),
            task: tokio::spawn(server),
        }
    }

    async fn stop(self) {
        self.handle.stop(true).await;
        self.task.await.unwrap().unwrap();
        self.jwks.stop().await;
    }

    async fn mcp(&self, token: &str, method: &str, params: Value) -> reqwest::Response {
        reqwest::Client::new()
            .post(format!("{}/mcp", self.address))
            .bearer_auth(token)
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", "2025-11-25")
            .json(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
            .send()
            .await
            .unwrap()
    }

    /// `structuredContent` of a successful tool call, or `Err` with the
    /// tool error text.
    async fn tool(&self, token: &str, name: &str, arguments: Value) -> Result<Value, String> {
        let params = json!({"name": name, "arguments": arguments});
        let response = self.mcp(token, "tools/call", params).await;
        assert_eq!(response.status(), 200);
        let body: Value = response.json().await.unwrap();
        let result = &body["result"];
        if result["isError"] == true {
            Err(result["content"][0]["text"].as_str().unwrap().to_string())
        } else {
            Ok(result["structuredContent"].clone())
        }
    }

    async fn mcp_can_replay(&self, token: &str, event_type: &str) -> bool {
        let arguments = json!({"event_type": event_type, "identifier": identifier(),
                               "from_id": "1"});
        match self
            .tool(token, "aviso_replay_notifications", arguments)
            .await
        {
            Ok(result) => {
                assert_eq!(result["count"], 1, "{event_type}");
                true
            }
            Err(message) => {
                assert_eq!(message, unavailable(event_type));
                false
            }
        }
    }

    async fn rest_can_replay(&self, token: &str, event_type: &str) -> bool {
        let response = reqwest::Client::new()
            .post(format!("{}/api/v1/replay", self.address))
            .bearer_auth(token)
            .json(
                &json!({"event_type": event_type, "identifier": identifier(),
                          "from_id": "1"}),
            )
            .send()
            .await
            .unwrap();
        match response.status().as_u16() {
            200 => true,
            403 => false,
            other => panic!("unexpected REST status {other} for {event_type}"),
        }
    }

    /// Replay every shared stream as `caller` over REST and over MCP and
    /// compare with the streams each side is expected to allow.
    async fn assert_reads(&self, caller: &Caller, rest: &[&str], mcp: &[&str]) {
        for event_type in SHARED_STREAMS {
            let who = &caller.label;
            let allowed = self.rest_can_replay(&caller.rest, event_type).await;
            assert_eq!(
                allowed,
                rest.contains(&event_type),
                "REST {who} {event_type}"
            );
            let allowed = self.mcp_can_replay(&caller.mcp, event_type).await;
            assert_eq!(allowed, mcp.contains(&event_type), "MCP {who} {event_type}");
        }
    }

    async fn listed(&self, token: &str) -> Value {
        let listed = self.tool(token, "aviso_list_event_types", json!({})).await;
        listed.unwrap()["event_types"].clone()
    }
}

fn identifier() -> Value {
    json!({"class": "od", "number": {"gte": 0}})
}

fn unavailable(event_type: &str) -> String {
    format!(
        "Event type '{event_type}' not found or not readable with this token; \
         aviso_list_event_types lists the readable ones"
    )
}

/// The same realm and roles as an MCP access token and as a REST JWT.
struct Caller {
    label: String,
    mcp: String,
    rest: String,
}

fn caller(realm: &str, roles: &[&str]) -> Caller {
    let mut claims = good_claims();
    claims["realm"] = json!(realm);
    claims["roles"] = json!(roles);
    let now = chrono::Utc::now().timestamp();
    let rest_claims = json!({
        "sub": "f00dbabe", "username": "f00dbabe", "iat": now, "exp": now + 3600,
        "realm": realm, "roles": roles
    });
    let rest = encode(
        &Header::default(),
        &rest_claims,
        &EncodingKey::from_secret(REST_SECRET.as_bytes()),
    )
    .unwrap();
    Caller {
        label: format!("{realm} {roles:?}"),
        mcp: mint(&claims, &key_a(), KID_A),
        rest,
    }
}

fn token_claims(admin: bool) -> Value {
    json!({"source": "token_claims", "admin": admin})
}

#[tokio::test]
async fn token_claims_reach_the_same_read_decisions_as_rest() {
    let server = Server::start(token_claims(true), admins(&["ecmwf"])).await;
    let cases: [(&str, &[&str], &[&str]); 8] = [
        ("ecmwf", &["reader"], &["authed", "ecmwf_readers", "open"]),
        ("ecmwf", &["viewer"], ANYONE),
        ("ecmwf", &["*"], ANYONE),
        ("ECMWF", &["admin", "reader"], ANYONE),
        (
            "browser-demo",
            &["reader"],
            &["authed", "demo_realm", "open"],
        ),
        ("browser-demo", &[], &["authed", "demo_realm", "open"]),
        (
            "browser-demo",
            &["admin"],
            &["authed", "demo_realm", "open"],
        ),
        ("ecmwf", &["admin"], EVERY_SHARED),
    ];
    for (realm, roles, readable) in cases {
        server
            .assert_reads(&caller(realm, roles), readable, readable)
            .await;
    }
    assert_eq!(server.jwks.hits(), 1, "one key set serves every caller");
    server.stop().await;
}

#[tokio::test]
async fn identity_claims_count_only_after_the_signature_verifies() {
    let server = Server::start(token_claims(true), admins(&["ecmwf"])).await;
    let mut claims = good_claims();
    claims["realm"] = json!("ecmwf");
    claims["roles"] = json!(["admin"]);
    for (token, reason) in [
        (
            mint(&claims, &key_b(), KID_A),
            "token is invalid: bad signature",
        ),
        (
            mint(&claims, &key_b(), KID_B),
            "token signing key is unknown",
        ),
    ] {
        let response = server.mcp(&token, "ping", json!({})).await;
        assert_eq!(response.status(), 401);
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["error_description"], reason);
    }
    server
        .jwks
        .set_keys(vec![jwk(&key_a(), KID_A), jwk(&key_b(), KID_B)]);
    let rotated = mint(&claims, &key_b(), KID_B);
    let describe = json!({"event_type": "plugged"});
    let described = server
        .tool(&rotated, "aviso_describe_event_type", describe)
        .await;
    assert!(described.is_ok(), "{described:?}");
    server.stop().await;
}

#[tokio::test]
async fn token_without_identity_claims_is_an_invalid_token() {
    let server = Server::start(token_claims(false), admins(&["ecmwf"])).await;
    let mut claims = good_claims();
    claims.as_object_mut().unwrap().remove("roles");
    let token = mint(&claims, &key_a(), KID_A);
    let response = server.mcp(&token, "ping", json!({})).await;
    assert_eq!(response.status(), 401);
    let challenge = response.headers()["www-authenticate"].to_str().unwrap();
    assert!(
        challenge.contains(
            "error=\"invalid_token\", error_description=\"token identity claims are \
             invalid: realm and roles claims are missing\""
        ),
        "{challenge}"
    );
    server.stop().await;
}

#[tokio::test]
async fn fixed_source_ignores_identity_claims_and_grants_no_admin() {
    let identity = json!({"source": "fixed", "realm": "mcp"});
    let server = Server::start(identity, admins(&["mcp", "ecmwf"])).await;
    let caller = caller("ecmwf", &["reader", "admin", "aviso-mcp"]);
    let expected = json!(["authed", "mcp_fixed", "open"]);
    assert_eq!(server.listed(&caller.mcp).await, expected);
    assert!(server.mcp_can_replay(&caller.mcp, "mcp_fixed").await);
    assert!(!server.mcp_can_replay(&caller.mcp, "ecmwf_readers").await);
    let describe = json!({"event_type": "plugged"});
    let described = server
        .tool(&caller.mcp, "aviso_describe_event_type", describe)
        .await;
    assert_eq!(described, Err(unavailable("plugged")));
    let without_claims = good_token();
    assert_eq!(server.listed(&without_claims).await, expected);
    server.stop().await;
}
