use crate::auth::client::{AuthClient, AuthClientError};
use crate::auth::{User, UserExtractionError, extract_bearer_token, validate_jwt};
use crate::configuration::{AuthMode, AuthSettings};
use crate::telemetry::{SERVICE_NAME, SERVICE_VERSION};
use actix_web::{
    Error, HttpMessage, HttpRequest, HttpResponse,
    body::{EitherBody, MessageBody},
    dev::{Service, ServiceRequest, ServiceResponse, Transform},
    http::{StatusCode, header},
};
use futures_util::future::{LocalBoxFuture, Ready, ready};
use serde_json::json;
use std::rc::Rc;
use std::sync::Arc;
use std::task::{Context, Poll};
use tracing::warn;

const WWW_AUTHENTICATE_BEARER: &str = "Bearer";
const WWW_AUTHENTICATE_BASIC: &str = "Basic";

#[derive(Debug, Clone)]
pub struct AuthContext {
    pub enabled: bool,
    pub mode: AuthMode,
}

#[derive(Clone)]
pub struct AuthMiddleware {
    settings: Arc<AuthSettings>,
    auth_client: Option<Arc<AuthClient>>,
}

impl AuthMiddleware {
    pub fn with_settings(settings: AuthSettings) -> Self {
        Self::with_arc_settings(Arc::new(settings))
    }

    pub fn with_arc_settings(settings: Arc<AuthSettings>) -> Self {
        let auth_client = if settings.enabled
            && settings.mode == AuthMode::Direct
            && !settings.auth_o_tron_url.trim().is_empty()
        {
            match AuthClient::new(&settings.auth_o_tron_url, settings.timeout_ms) {
                Ok(client) => Some(Arc::new(client)),
                Err(error) => {
                    warn!(
                        service_name = SERVICE_NAME,
                        service_version = SERVICE_VERSION,
                        event_name = "auth.middleware.client.initialization.failed",
                        outcome = "error",
                        error = %error,
                        "Failed to initialize auth-o-tron client; authenticated requests will fail closed"
                    );
                    None
                }
            }
        } else {
            None
        };

        Self {
            settings,
            auth_client,
        }
    }
}

impl<S, B> Transform<S, ServiceRequest> for AuthMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type Transform = AuthMiddlewareService<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(AuthMiddlewareService {
            service: Rc::new(service),
            settings: Arc::clone(&self.settings),
            auth_client: self.auth_client.clone(),
        }))
    }
}

pub struct AuthMiddlewareService<S> {
    service: Rc<S>,
    settings: Arc<AuthSettings>,
    auth_client: Option<Arc<AuthClient>>,
}

impl<S, B> Service<ServiceRequest> for AuthMiddlewareService<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let service = Rc::clone(&self.service);
        let settings = Arc::clone(&self.settings);
        let auth_client = self.auth_client.clone();

        Box::pin(async move {
            if !settings.enabled {
                req.extensions_mut().insert(AuthContext {
                    enabled: false,
                    mode: settings.mode,
                });
                return service
                    .call(req)
                    .await
                    .map(ServiceResponse::map_into_left_body);
            }

            req.extensions_mut().insert(AuthContext {
                enabled: true,
                mode: settings.mode,
            });

            // Schema endpoints are always public — skip auth entirely so
            // malformed headers or auth-o-tron outages cannot block them.
            if req.path().starts_with("/api/v1/schema") {
                return service
                    .call(req)
                    .await
                    .map(ServiceResponse::map_into_left_body);
            }

            let is_admin_path = req.path().starts_with("/api/v1/admin");
            let user =
                match resolve_user(&req, &settings, auth_client.as_deref(), is_admin_path).await {
                    Ok(user) => user,
                    Err(failure) => {
                        log_failure(req.path(), &failure);
                        return Ok(req
                            .into_response(failure.response(settings.mode).map_into_right_body()));
                    }
                };

            if let Some(user) = user {
                if is_admin_path && !user.is_admin(&settings.admin_roles) {
                    let failure =
                        AuthFailure::Forbidden("User does not have required administrative role");
                    log_failure(req.path(), &failure);
                    return Ok(
                        req.into_response(failure.response(settings.mode).map_into_right_body())
                    );
                }
                req.extensions_mut().insert(user);
            }

            service
                .call(req)
                .await
                .map(ServiceResponse::map_into_left_body)
        })
    }
}

pub fn get_user(req: &HttpRequest) -> Option<User> {
    req.extensions().get::<User>().cloned()
}

pub fn is_auth_enabled(req: &HttpRequest) -> bool {
    req.extensions()
        .get::<AuthContext>()
        .is_some_and(|context| context.enabled)
}

pub fn auth_mode(req: &HttpRequest) -> Option<AuthMode> {
    req.extensions()
        .get::<AuthContext>()
        .map(|context| context.mode)
}

#[derive(Debug)]
enum AuthFailure {
    Unauthorized(&'static str),
    Forbidden(&'static str),
    ServiceUnavailable(&'static str),
}

impl AuthFailure {
    fn status_code(&self) -> StatusCode {
        match self {
            Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::ServiceUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
        }
    }

    fn reason(&self) -> &'static str {
        match self {
            Self::Unauthorized(_) => "unauthorized",
            Self::Forbidden(_) => "forbidden",
            Self::ServiceUnavailable(_) => "service_unavailable",
        }
    }

    fn message(&self) -> &'static str {
        match self {
            Self::Unauthorized(message)
            | Self::Forbidden(message)
            | Self::ServiceUnavailable(message) => message,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::Unauthorized(_) => "UNAUTHORIZED",
            Self::Forbidden(_) => "FORBIDDEN",
            Self::ServiceUnavailable(_) => "SERVICE_UNAVAILABLE",
        }
    }

    fn response(&self, auth_mode: AuthMode) -> HttpResponse {
        if matches!(self, Self::Unauthorized(_)) {
            return unauthorized_response(auth_mode, self.message());
        }

        HttpResponse::build(self.status_code()).json(json!({
            "code": self.code(),
            "error": self.reason(),
            "message": self.message()
        }))
    }
}

pub fn unauthorized_response(auth_mode: AuthMode, message: &str) -> HttpResponse {
    let mut response = HttpResponse::Unauthorized();
    append_www_authenticate_headers(&mut response, auth_mode);
    response.json(json!({
        "code": "UNAUTHORIZED",
        "error": "unauthorized",
        "message": message
    }))
}

fn append_www_authenticate_headers(
    response: &mut actix_web::HttpResponseBuilder,
    auth_mode: AuthMode,
) {
    response.append_header((header::WWW_AUTHENTICATE, WWW_AUTHENTICATE_BEARER));
    if auth_mode == AuthMode::Direct {
        response.append_header((header::WWW_AUTHENTICATE, WWW_AUTHENTICATE_BASIC));
    }
}

fn log_failure(path: &str, failure: &AuthFailure) {
    warn!(
        service_name = SERVICE_NAME,
        service_version = SERVICE_VERSION,
        event_name = "auth.middleware.request.denied",
        outcome = "error",
        status_code = failure.status_code().as_u16(),
        reason = failure.reason(),
        path = path,
        "Request denied by auth middleware"
    );
}

async fn authenticate_user(
    credentials: &str,
    settings: &AuthSettings,
    auth_client: Option<&AuthClient>,
) -> Result<User, AuthFailure> {
    let Some(client) = auth_client else {
        return Err(AuthFailure::ServiceUnavailable(
            "Authentication service is not configured correctly",
        ));
    };

    match client.authenticate(credentials).await {
        Ok(authenticated_token) => user_from_remote_token(&authenticated_token, settings),
        Err(AuthClientError::Unauthorized) => {
            Err(AuthFailure::Unauthorized("Invalid or expired token"))
        }
        Err(AuthClientError::Forbidden) => {
            Err(AuthFailure::Forbidden("Token validation was denied"))
        }
        Err(error) => {
            warn!(
                service_name = SERVICE_NAME,
                service_version = SERVICE_VERSION,
                event_name = "auth.middleware.remote.validation.failed",
                outcome = "error",
                error = %error,
                "auth-o-tron validation failed; authentication service unavailable"
            );
            Err(AuthFailure::ServiceUnavailable(
                "Authentication service temporarily unavailable",
            ))
        }
    }
}

async fn resolve_user(
    req: &ServiceRequest,
    settings: &AuthSettings,
    auth_client: Option<&AuthClient>,
    is_admin_path: bool,
) -> Result<Option<User>, AuthFailure> {
    // Keep credential extraction/authentication mode-specific, then apply a
    // single admin-route rule for missing credentials across both modes.
    let user = match settings.mode {
        AuthMode::Direct => match resolve_direct_credentials(req)? {
            Some(credentials) => {
                Some(authenticate_user(&credentials, settings, auth_client).await?)
            }
            None => None,
        },
        AuthMode::TrustedProxy => match resolve_trusted_proxy_token(req)? {
            Some(token) => Some(user_from_remote_token(&token, settings)?),
            None => None,
        },
    };

    if user.is_none() && is_admin_path {
        return Err(AuthFailure::Unauthorized(
            "Authorization header is required",
        ));
    }

    Ok(user)
}

fn resolve_direct_credentials(req: &ServiceRequest) -> Result<Option<String>, AuthFailure> {
    if let Some(authorization) = req.headers().get(header::AUTHORIZATION) {
        let value = authorization
            .to_str()
            .map_err(|_| AuthFailure::Unauthorized("Authorization header is not valid UTF-8"))?
            .trim();
        if value.is_empty() {
            return Err(AuthFailure::Unauthorized(
                "Authorization header cannot be empty",
            ));
        }
        let Some((scheme, credentials)) = value.split_once(' ') else {
            return Err(AuthFailure::Unauthorized(
                "Authorization header must use Bearer or Basic scheme",
            ));
        };
        if !scheme.eq_ignore_ascii_case("Bearer") && !scheme.eq_ignore_ascii_case("Basic") {
            return Err(AuthFailure::Unauthorized(
                "Authorization header must use Bearer or Basic scheme",
            ));
        }
        if credentials.trim().is_empty() {
            return Err(AuthFailure::Unauthorized(
                "Authorization header credentials cannot be empty",
            ));
        }
        return Ok(Some(value.to_string()));
    }

    Ok(None)
}

fn resolve_trusted_proxy_token(req: &ServiceRequest) -> Result<Option<String>, AuthFailure> {
    let Some(authorization) = req.headers().get(header::AUTHORIZATION) else {
        return Ok(None);
    };

    let token = extract_bearer_token(authorization).ok_or(AuthFailure::Unauthorized(
        "Authorization header must use Bearer token",
    ))?;
    if token.trim().is_empty() {
        return Err(AuthFailure::Unauthorized(
            "Authorization header cannot contain an empty token",
        ));
    }
    Ok(Some(token.to_string()))
}

fn user_from_remote_token(token: &str, settings: &AuthSettings) -> Result<User, AuthFailure> {
    let claims = validate_jwt(token, &settings.jwt_secret)
        .map_err(|_| AuthFailure::Unauthorized("Invalid or expired token"))?;

    User::try_from(claims).map_err(map_user_extraction_error)
}

fn map_user_extraction_error(error: UserExtractionError) -> AuthFailure {
    match error {
        UserExtractionError::MissingUsername => {
            AuthFailure::Unauthorized("Token is missing required user claims")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::JwtClaims;
    use actix_web::{
        App, HttpMessage, HttpResponse,
        dev::ServiceRequest,
        http::{
            StatusCode,
            header::{AUTHORIZATION, WWW_AUTHENTICATE},
        },
        test, web,
    };
    use chrono::Utc;
    use jsonwebtoken::{EncodingKey, Header, encode};
    use serde_json::{Value, json};
    use std::collections::HashMap;

    fn local_auth_settings(secret: &str) -> AuthSettings {
        AuthSettings {
            enabled: true,
            mode: AuthMode::Direct,
            auth_o_tron_url: "http://127.0.0.1:9".to_string(),
            jwt_secret: secret.to_string(),
            admin_roles: HashMap::from([("testrealm".to_string(), vec!["admin".to_string()])]),
            timeout_ms: 5_000,
        }
    }

    fn request_with_headers(path: &str, headers: &[(&str, &str)]) -> ServiceRequest {
        let mut request = test::TestRequest::get().uri(path);
        for (name, value) in headers {
            request = request.insert_header((*name, *value));
        }
        request.to_srv_request()
    }

    fn signed_token(secret: &str, username: &str, roles: &[&str]) -> String {
        let claims = JwtClaims {
            sub: Some(username.to_string()),
            iss: Some("tests".to_string()),
            exp: (Utc::now().timestamp() + 3_600) as usize,
            iat: Some(Utc::now().timestamp() as usize),
            username: Some(username.to_string()),
            realm: Some("testrealm".to_string()),
            roles: roles.iter().map(|role| (*role).to_string()).collect(),
            attributes: HashMap::new(),
        };

        encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .expect("token must encode")
    }

    async fn inspect_user(req: HttpRequest) -> HttpResponse {
        let body = match req.extensions().get::<User>() {
            Some(user) => json!({
                "username": user.username,
                "roles": user.roles,
            }),
            None => json!({
                "username": Value::Null,
                "roles": Value::Array(vec![]),
            }),
        };
        HttpResponse::Ok().json(body)
    }

    #[actix_web::test]
    async fn allows_request_when_auth_is_disabled() {
        let app = test::init_service(
            App::new()
                .wrap(AuthMiddleware::with_settings(AuthSettings::default()))
                .route("/api/v1/watch", web::get().to(inspect_user)),
        )
        .await;

        let response = test::call_service(
            &app,
            test::TestRequest::get().uri("/api/v1/watch").to_request(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[actix_web::test]
    async fn allows_missing_authorization_header_on_non_admin_route_when_auth_is_enabled() {
        let app = test::init_service(
            App::new()
                .wrap(AuthMiddleware::with_settings(local_auth_settings("secret")))
                .route("/api/v1/watch", web::get().to(inspect_user)),
        )
        .await;

        let response = test::call_service(
            &app,
            test::TestRequest::get().uri("/api/v1/watch").to_request(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[actix_web::test]
    async fn fails_token_validation_when_auth_service_is_unavailable() {
        let app = test::init_service(
            App::new()
                .wrap(AuthMiddleware::with_settings(local_auth_settings("secret")))
                .route("/api/v1/watch", web::get().to(inspect_user)),
        )
        .await;

        let request = test::TestRequest::get()
            .uri("/api/v1/watch")
            .insert_header((AUTHORIZATION, "Bearer not-a-jwt"))
            .to_request();
        let response = test::call_service(&app, request).await;

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[actix_web::test]
    async fn fails_authenticated_request_when_auth_service_is_unavailable() {
        let secret = "secret";
        let token = signed_token(secret, "alice", &["reader"]);

        let app = test::init_service(
            App::new()
                .wrap(AuthMiddleware::with_settings(local_auth_settings(secret)))
                .route("/api/v1/watch", web::get().to(inspect_user)),
        )
        .await;

        let request = test::TestRequest::get()
            .uri("/api/v1/watch")
            .insert_header((AUTHORIZATION, format!("Bearer {token}")))
            .to_request();
        let response = test::call_service(&app, request).await;

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[actix_web::test]
    async fn admin_route_token_auth_fails_when_auth_service_is_unavailable() {
        let secret = "secret";
        let token = signed_token(secret, "bob", &["reader"]);

        let app = test::init_service(
            App::new()
                .wrap(AuthMiddleware::with_settings(local_auth_settings(secret)))
                .route("/api/v1/admin/wipe", web::delete().to(inspect_user)),
        )
        .await;

        let request = test::TestRequest::delete()
            .uri("/api/v1/admin/wipe")
            .insert_header((AUTHORIZATION, format!("Bearer {token}")))
            .to_request();
        let response = test::call_service(&app, request).await;

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[actix_web::test]
    async fn rejects_admin_route_without_authorization_header() {
        let app = test::init_service(
            App::new()
                .wrap(AuthMiddleware::with_settings(local_auth_settings("secret")))
                .route("/api/v1/admin/wipe", web::delete().to(inspect_user)),
        )
        .await;

        let response = test::call_service(
            &app,
            test::TestRequest::delete()
                .uri("/api/v1/admin/wipe")
                .to_request(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let challenges: Vec<_> = response
            .headers()
            .get_all(WWW_AUTHENTICATE)
            .filter_map(|value| value.to_str().ok())
            .collect();
        assert_eq!(challenges, vec!["Bearer", "Basic"]);
    }

    #[actix_web::test]
    async fn resolves_direct_credentials_from_authorization_header() {
        let request =
            request_with_headers("/api/v1/watch", &[("Authorization", "Basic dXNlcjpwYXNz")]);
        let credentials = resolve_direct_credentials(&request).expect("credentials should resolve");
        assert_eq!(credentials.as_deref(), Some("Basic dXNlcjpwYXNz"));
    }

    #[actix_web::test]
    async fn direct_mode_rejects_non_basic_or_bearer_authorization() {
        let request = request_with_headers("/api/v1/watch", &[("Authorization", "Digest abc")]);
        let err = resolve_direct_credentials(&request)
            .expect_err("non-basic/bearer direct credentials should fail");
        assert_eq!(err.status_code(), StatusCode::UNAUTHORIZED);
    }

    #[actix_web::test]
    async fn trusted_proxy_rejects_non_bearer_authorization() {
        let request =
            request_with_headers("/api/v1/watch", &[("Authorization", "Basic dXNlcjpwYXNz")]);

        let err =
            resolve_trusted_proxy_token(&request).expect_err("trusted proxy token should fail");
        assert_eq!(err.status_code(), StatusCode::UNAUTHORIZED);
    }

    #[actix_web::test]
    async fn trusted_proxy_resolves_bearer_token() {
        let request = request_with_headers("/api/v1/watch", &[("Authorization", "Bearer a.b.c")]);

        let token = resolve_trusted_proxy_token(&request)
            .expect("trusted proxy token extraction should succeed")
            .expect("token should be present");
        assert_eq!(token, "a.b.c");
    }

    #[actix_web::test]
    async fn trusted_proxy_unauthorized_response_advertises_bearer_only() {
        let mut settings = local_auth_settings("secret");
        settings.mode = AuthMode::TrustedProxy;

        let app = test::init_service(
            App::new()
                .wrap(AuthMiddleware::with_settings(settings))
                .route("/api/v1/admin/wipe", web::delete().to(inspect_user)),
        )
        .await;

        let response = test::call_service(
            &app,
            test::TestRequest::delete()
                .uri("/api/v1/admin/wipe")
                .to_request(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let challenges: Vec<_> = response
            .headers()
            .get_all(WWW_AUTHENTICATE)
            .filter_map(|value| value.to_str().ok())
            .collect();
        assert_eq!(challenges, vec!["Bearer"]);
    }
}
