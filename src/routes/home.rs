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
