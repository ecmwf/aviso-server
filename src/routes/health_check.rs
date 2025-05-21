use actix_web::HttpResponse;
use serde::Serialize;

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

pub async fn health_check() -> HttpResponse {
    HttpResponse::Ok().json(HealthResponse { status: "ok" })
}
