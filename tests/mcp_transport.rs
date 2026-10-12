// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Streamable HTTP rules of `/mcp` and the protected resource metadata.
//!
//! The ignored interop test drives a real server with the official MCP
//! Python SDK. It needs Python with the `mcp` package:
//!
//! ```text
//! MCP_INTEROP_PYTHON=/path/to/venv/bin/python \
//!   cargo test --test mcp_transport -- --ignored
//! ```

#[path = "common/mcp_issuer.rs"]
mod mcp_issuer;

use actix_web::dev::ServiceResponse;
use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpServer, test, web};
use aviso_server::configuration::Settings;
use aviso_server::mcp::auth::TokenVerifier;
use aviso_server::mcp::jwks::{JwksCache, JwksTiming};
use aviso_server::mcp::state::McpState;
use aviso_server::notification_backend::{NotificationBackend, build_backend};
use mcp_issuer::*;
use serde_json::{Value, json};
use std::process::Command;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

const METADATA: &str = "https://aviso.example.int/.well-known/oauth-protected-resource/mcp";

fn init_config() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        let config: Settings = serde_json::from_value(json!({
            "application": {"host": "127.0.0.1", "port": 0},
            "notification_backend": {"kind": "in_memory"},
            "notification_schema": {
                "open": {
                    "topic": {"base": "McpTransport", "key_order": ["class"]},
                    "identifier": {"class": {"type": "StringHandler", "required": true}},
                    "payload": {"required": false}
                },
                "interop": {
                    "topic": {"base": "McpInterop", "key_order": ["class", "number"]},
                    "identifier": {
                        "class": {"type": "StringHandler", "required": true},
                        "number": {"type": "IntHandler", "required": true}
                    },
                    "payload": {"required": false}
                }
            }
        }))
        .unwrap();
        config.init_global_config();
    });
}

async fn state() -> (
    JwksServer,
    web::Data<McpState>,
    Arc<dyn NotificationBackend>,
) {
    init_config();
    let jwks = JwksServer::start(vec![jwk(&key_a(), KID_A)]);
    let settings = settings(&jwks.uri);
    let timing = JwksTiming {
        min_refetch_interval: Duration::ZERO,
        ..JwksTiming::with_ttl(Duration::from_secs(300))
    };
    let cache = JwksCache::new(&jwks.uri, timing).unwrap();
    let state = McpState::new(&settings, TokenVerifier::new(cache, &settings)).unwrap();
    let backend = build_backend(&serde_json::from_value(json!({"kind": "in_memory"})).unwrap())
        .await
        .unwrap();
    (jwks, web::Data::new(state), backend)
}

/// The MCP routes over an empty in-memory backend, plus the issuer.
macro_rules! harness {
    () => {{
        let (jwks, state, backend) = state().await;
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(backend))
                .configure(move |cfg| aviso_server::mcp::configure(cfg, state.clone())),
        )
        .await;
        (jwks, app)
    }};
}

fn rpc(id: i64, method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

fn post(body: &Value) -> test::TestRequest {
    test::TestRequest::post()
        .uri("/mcp")
        .insert_header((header::AUTHORIZATION, format!("Bearer {}", good_token())))
        .insert_header((header::ACCEPT, "application/json, text/event-stream"))
        .insert_header((header::CONTENT_TYPE, "application/json"))
        .insert_header(("MCP-Protocol-Version", "2025-11-25"))
        .set_payload(body.to_string())
}

fn challenge(response: &ServiceResponse) -> String {
    response
        .headers()
        .get(header::WWW_AUTHENTICATE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string()
}

#[actix_web::test]
async fn missing_token_gets_a_metadata_challenge_before_body_parsing() {
    let (jwks, app) = harness!();
    let request = test::TestRequest::post()
        .uri("/mcp")
        .set_payload("{not json")
        .to_request();
    let response = test::call_service(&app, request).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        challenge(&response),
        format!("Bearer resource_metadata=\"{METADATA}\", scope=\"aviso:read\"")
    );
    assert_eq!(jwks.hits(), 0, "no key fetch without a token");
}

#[actix_web::test]
async fn invalid_token_and_missing_scope_are_distinguished() {
    let (_jwks, app) = harness!();
    let request = post(&rpc(1, "ping", json!({})))
        .insert_header((header::AUTHORIZATION, "Bearer not.a.token"))
        .to_request();
    let response = test::call_service(&app, request).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(challenge(&response).starts_with("Bearer error=\"invalid_token\", resource_metadata="));

    let mut claims = good_claims();
    claims["scope"] = json!("openid");
    let token = mint(&claims, &key_a(), KID_A);
    let request = post(&rpc(1, "ping", json!({})))
        .insert_header((header::AUTHORIZATION, format!("Bearer {token}")))
        .to_request();
    let response = test::call_service(&app, request).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(challenge(&response).contains("error=\"insufficient_scope\""));
    assert!(challenge(&response).contains("scope=\"aviso:read\""));
}

#[actix_web::test]
async fn origin_must_be_absent_or_allowed() {
    let (_jwks, app) = harness!();
    let denied = post(&rpc(1, "ping", json!({})))
        .insert_header((header::ORIGIN, "https://evil.example"))
        .to_request();
    assert_eq!(
        test::call_service(&app, denied).await.status(),
        StatusCode::FORBIDDEN
    );
    let allowed = post(&rpc(1, "ping", json!({})))
        .insert_header((header::ORIGIN, "http://localhost:6274"))
        .to_request();
    let response = test::call_service(&app, allowed).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .unwrap(),
        "http://localhost:6274"
    );
    let preflight = test::TestRequest::default()
        .method(Method::OPTIONS)
        .uri("/mcp")
        .insert_header((header::ORIGIN, "http://localhost:6274"))
        .to_request();
    let response = test::call_service(&app, preflight).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(
        response
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_HEADERS)
            .unwrap()
            .to_str()
            .unwrap()
            .contains("authorization")
    );
}

#[actix_web::test]
async fn get_and_delete_are_not_allowed() {
    let (_jwks, app) = harness!();
    for method in [Method::GET, Method::DELETE, Method::PUT] {
        let request = test::TestRequest::default()
            .method(method.clone())
            .uri("/mcp")
            .to_request();
        let response = test::call_service(&app, request).await;
        assert_eq!(
            response.status(),
            StatusCode::METHOD_NOT_ALLOWED,
            "{method}"
        );
        assert_eq!(response.headers().get(header::ALLOW).unwrap(), "POST");
    }
}

#[actix_web::test]
async fn accept_and_content_type_are_checked() {
    let (_jwks, app) = harness!();
    for (accept, status) in [
        ("text/event-stream", StatusCode::NOT_ACCEPTABLE),
        ("application/json", StatusCode::OK),
        ("*/*", StatusCode::OK),
    ] {
        let request = post(&rpc(1, "ping", json!({})))
            .insert_header((header::ACCEPT, accept))
            .to_request();
        assert_eq!(
            test::call_service(&app, request).await.status(),
            status,
            "{accept}"
        );
    }
    let mut request = post(&rpc(1, "ping", json!({}))).to_request();
    request.headers_mut().remove(header::ACCEPT);
    assert_eq!(
        test::call_service(&app, request).await.status(),
        StatusCode::NOT_ACCEPTABLE
    );
    let request = post(&rpc(1, "ping", json!({})))
        .insert_header((header::CONTENT_TYPE, "text/plain"))
        .to_request();
    assert_eq!(
        test::call_service(&app, request).await.status(),
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
}

#[actix_web::test]
async fn protocol_version_header_is_validated() {
    let (_jwks, app) = harness!();
    for version in ["2025-11-25", "2025-06-18", "2025-03-26"] {
        let request = post(&rpc(1, "ping", json!({})))
            .insert_header(("MCP-Protocol-Version", version))
            .to_request();
        assert_eq!(
            test::call_service(&app, request).await.status(),
            StatusCode::OK
        );
    }
    let mut request = post(&rpc(1, "ping", json!({}))).to_request();
    request.headers_mut().remove("mcp-protocol-version");
    assert_eq!(
        test::call_service(&app, request).await.status(),
        StatusCode::OK
    );
    let request = post(&rpc(1, "ping", json!({})))
        .insert_header(("MCP-Protocol-Version", "2024-11-05"))
        .to_request();
    assert_eq!(
        test::call_service(&app, request).await.status(),
        StatusCode::BAD_REQUEST
    );
}

#[actix_web::test]
async fn initialize_negotiates_and_never_opens_a_session() {
    let (_jwks, app) = harness!();
    for (requested, expected) in [("2025-06-18", "2025-06-18"), ("1999-01-01", "2025-11-25")] {
        let params = json!({
            "protocolVersion": requested,
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "0"}
        });
        let request = post(&rpc(7, "initialize", params)).to_request();
        let response = test::call_service(&app, request).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().get("mcp-session-id").is_none());
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/json"
        );
        let body: Value = test::read_body_json(response).await;
        assert_eq!(body["id"], 7);
        assert_eq!(body["result"]["protocolVersion"], expected);
        assert_eq!(body["result"]["serverInfo"]["name"], "aviso-server");
        assert_eq!(
            body["result"]["capabilities"],
            json!({"tools": {"listChanged": false}})
        );
    }
    let request = post(&rpc(8, "initialize", json!({}))).to_request();
    let body: Value = test::call_and_read_body_json(&app, request).await;
    assert_eq!(body["error"]["code"], -32602);
}

#[actix_web::test]
async fn notifications_and_responses_are_accepted_without_body() {
    let (_jwks, app) = harness!();
    for message in [
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        json!({"jsonrpc": "2.0", "id": 4, "result": {}}),
    ] {
        let response = test::call_service(&app, post(&message).to_request()).await;
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        assert!(test::read_body(response).await.is_empty());
    }
}

#[actix_web::test]
async fn malformed_messages_get_json_rpc_errors() {
    let (_jwks, app) = harness!();
    let batch = json!([rpc(1, "ping", json!({}))]);
    let response = test::call_service(&app, post(&batch).to_request()).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body: Value = test::read_body_json(response).await;
    assert_eq!(body["error"]["code"], -32600);
    assert_eq!(body["id"], Value::Null);

    let request = post(&json!({})).set_payload("{\"jsonrpc\": ").to_request();
    let body: Value = test::read_body_json(test::call_service(&app, request).await).await;
    assert_eq!(body["error"]["code"], -32700);

    for (message, code) in [
        (rpc(2, "resources/list", json!({})), -32601),
        (rpc(3, "tools/call", json!({})), -32602),
        (rpc(4, "tools/call", json!({"name": "aviso_write"})), -32602),
    ] {
        let response = test::call_service(&app, post(&message).to_request()).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value = test::read_body_json(response).await;
        assert_eq!(body["error"]["code"], code, "{message}");
    }
}

#[actix_web::test]
async fn oversized_body_is_rejected() {
    let (_jwks, app) = harness!();
    let request = post(&json!({}))
        .set_payload(vec![b' '; 65 * 1024])
        .to_request();
    assert_eq!(
        test::call_service(&app, request).await.status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
}

#[actix_web::test]
async fn tools_are_listed_and_callable() {
    let (_jwks, app) = harness!();
    let body: Value =
        test::call_and_read_body_json(&app, post(&rpc(1, "tools/list", json!({}))).to_request())
            .await;
    assert_eq!(body["result"]["tools"].as_array().unwrap().len(), 5);
    let call = rpc(2, "tools/call", json!({"name": "aviso_list_event_types"}));
    let body: Value = test::call_and_read_body_json(&app, post(&call).to_request()).await;
    assert_eq!(body["result"]["isError"], false);
    assert_eq!(
        body["result"]["structuredContent"]["event_types"],
        json!(["interop", "open"])
    );
}

#[actix_web::test]
async fn protected_resource_metadata_is_public() {
    let (_jwks, app) = harness!();
    for path in [
        "/.well-known/oauth-protected-resource/mcp",
        "/.well-known/oauth-protected-resource",
    ] {
        let request = test::TestRequest::get().uri(path).to_request();
        let body: Value = test::call_and_read_body_json(&app, request).await;
        assert_eq!(body["resource"], RESOURCE);
        assert_eq!(body["authorization_servers"], json!([ISSUER]));
        assert_eq!(body["scopes_supported"], json!(["aviso:read"]));
        assert_eq!(body["bearer_methods_supported"], json!(["header"]));
    }
}

#[actix_web::test]
async fn key_rotation_is_picked_up_and_unknown_keys_are_refused() {
    let (jwks, app) = harness!();
    let rotated = mint(&good_claims(), &key_b(), KID_B);
    let request = post(&rpc(1, "ping", json!({})))
        .insert_header((header::AUTHORIZATION, format!("Bearer {rotated}")))
        .to_request();
    let response = test::call_service(&app, request).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(challenge(&response).contains("error=\"invalid_token\""));

    jwks.set_keys(vec![jwk(&key_a(), KID_A), jwk(&key_b(), KID_B)]);
    let request = post(&rpc(2, "ping", json!({})))
        .insert_header((header::AUTHORIZATION, format!("Bearer {rotated}")))
        .to_request();
    assert_eq!(
        test::call_service(&app, request).await.status(),
        StatusCode::OK
    );
    jwks.stop().await;
}

#[tokio::test]
#[ignore = "needs Python with the mcp package; see the module docs"]
async fn python_sdk_client_completes_a_session() {
    let (jwks, state, backend) = state().await;
    for (class, number) in [("od", 1), ("rd", 2), ("od", 3)] {
        backend
            .put_messages(
                &format!("McpInterop.{class}.{number}"),
                json!({"number": number}).to_string(),
            )
            .await
            .unwrap();
    }

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let server = HttpServer::new(move || {
        let state = state.clone();
        App::new()
            .app_data(web::Data::new(backend.clone()))
            .configure(move |cfg| aviso_server::mcp::configure(cfg, state))
    })
    .workers(1)
    .listen(listener)
    .unwrap()
    .run();
    let handle = server.handle();
    tokio::spawn(server);

    let python = std::env::var("MCP_INTEROP_PYTHON").unwrap_or_else(|_| "python3".to_string());
    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/interop/mcp_python_client.py"
    );
    let token = good_token();
    let output = tokio::task::spawn_blocking(move || {
        Command::new(python)
            .arg(script)
            .env("MCP_URL", url)
            .env("MCP_TOKEN", token)
            .env("MCP_RESOURCE", RESOURCE)
            .output()
            .unwrap()
    })
    .await
    .unwrap();

    handle.stop(true).await;
    assert_eq!(jwks.hits(), 1, "keys are fetched once and cached");
    jwks.stop().await;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stdout.contains("interop ok"), "{stdout}");
}
