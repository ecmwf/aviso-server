use actix_web::{HttpResponse, Result};

pub async fn homepage() -> Result<HttpResponse> {
    let html = include_str!("../static/index.html"); // Or embed the HTML directly
    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(html))
}
