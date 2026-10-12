// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! OAuth 2.0 Protected Resource Metadata (RFC 9728) for `/mcp`.

use crate::mcp::state::McpState;
use actix_web::http::header;
use actix_web::{HttpResponse, web};
use serde_json::{Value, json};

/// The metadata document. `resource` and `authorization_servers` repeat
/// the configured strings byte for byte, because clients compare them
/// exactly.
pub fn document(state: &McpState) -> Value {
    json!({
        "resource": state.settings.resource_url,
        "authorization_servers": [state.settings.oauth.issuer],
        "scopes_supported": state.settings.oauth.required_scopes,
        "bearer_methods_supported": ["header"],
        "resource_name": "Aviso"
    })
}

/// `GET /.well-known/oauth-protected-resource[/mcp]`. Public, cacheable,
/// and readable from any origin.
pub async fn protected_resource_metadata(state: web::Data<McpState>) -> HttpResponse {
    HttpResponse::Ok()
        .insert_header((header::CACHE_CONTROL, "public, max-age=300"))
        .insert_header((header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"))
        .json(document(&state))
}
