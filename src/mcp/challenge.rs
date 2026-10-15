// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! RFC 6750 bearer challenges for `/mcp`. Every challenge points at the
//! RFC 9728 protected resource metadata, so clients can find the issuer.

use crate::mcp::state::McpState;
use actix_web::http::StatusCode;
use serde_json::{Value, json};

/// A refusal that asks for a (better) bearer token.
pub(super) struct Challenge {
    /// `401` or `403`.
    pub(super) status: StatusCode,
    /// Value of the `WWW-Authenticate` header.
    pub(super) header: String,
    /// JSON body of the response.
    pub(super) body: Value,
}

/// RFC 6750 and RFC 9728 challenge pointing at the resource metadata.
///
/// - no token: `Bearer resource_metadata="...", scope="aviso:read"`
/// - bad token: `Bearer error="invalid_token", resource_metadata="...", ...`
pub(super) fn challenge(state: &McpState, status: StatusCode, error: Option<&str>) -> Challenge {
    let error_part = error
        .map(|error| format!("error=\"{error}\", "))
        .unwrap_or_default();
    let header = format!(
        "Bearer {error_part}resource_metadata=\"{}\", scope=\"{}\"",
        state.metadata_url,
        state.verifier.required_scope_string()
    );
    Challenge {
        status,
        header,
        body: json!({"error": error.unwrap_or("unauthorized")}),
    }
}
