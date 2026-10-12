// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Read-only MCP tools over the Aviso schema and notification history.

pub mod definitions;

use serde_json::{Value, json};

/// Result of `tools/list`.
pub fn list() -> Value {
    json!({"tools": definitions::all()})
}
