// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Model Context Protocol endpoint: read-only Aviso tools for MCP clients,
//! protected by OAuth access tokens from an external issuer.

pub mod auth;
pub mod dispatch;
pub mod jwks;
pub mod metadata;
pub mod protocol;
pub mod state;
pub mod tools;
pub mod transport;

use crate::configuration::MCP_PATH;
use actix_web::web;
use state::{METADATA_PREFIX, McpState};

/// Mount `/mcp` and its metadata routes. Call only when MCP is enabled.
///
/// The routes sit outside `/api/v1`, so the REST auth middleware does not
/// apply; `/mcp` checks its own bearer tokens.
pub fn configure(cfg: &mut web::ServiceConfig, state: web::Data<McpState>) {
    let metadata = || web::get().to(metadata::protected_resource_metadata);
    cfg.service(
        web::resource(MCP_PATH)
            .app_data(state.clone())
            .route(web::post().to(transport::post_mcp))
            .default_service(web::to(transport::method_not_allowed)),
    )
    .service(
        web::resource(format!("{METADATA_PREFIX}{MCP_PATH}"))
            .app_data(state.clone())
            .route(metadata()),
    )
    .service(
        web::resource(METADATA_PREFIX)
            .app_data(state)
            .route(metadata()),
    );
}
