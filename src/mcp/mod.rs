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
