// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use crate::configuration::Settings;
use actix_web::{HttpResponse, Result};
use std::fs;
use std::path::PathBuf;

#[utoipa::path(
    get,
    path = "/",
    tag = "general",
    responses(
        (status = 200, description = "Homepage HTML content", content_type = "text/html")
    )
)]
pub async fn homepage() -> Result<HttpResponse> {
    let static_files_path = &Settings::get_global_application_settings().static_files_path;
    let mut index_path = PathBuf::from(static_files_path);
    index_path.push("index.html");

    let html = fs::read_to_string(index_path).unwrap_or_else(|_| {
        "<h1>Index file not found</h1><p>Please check the static files configuration.</p>"
            .to_string()
    });

    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(html))
}
