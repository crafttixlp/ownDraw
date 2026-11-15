//! Einheitliche API-Fehler: HTTP-Status + `{"error":"code","message":"Text"}`.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{json, Value};

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    pub extra: Option<Value>,
}

pub type ApiResult<T> = Result<T, ApiError>;

impl ApiError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self { status, code, message: message.into(), extra: None }
    }
    pub fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", "Bitte anmelden.")
    }
    pub fn unauthorized_with(m: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", m)
    }
    pub fn forbidden(m: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden", m)
    }
    pub fn license_required(m: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, "license_required", m)
    }
    pub fn not_found(m: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", m)
    }
    pub fn conflict(m: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, "conflict", m)
    }
    pub fn invalid(m: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid", m)
    }
    pub fn too_large(m: impl Into<String>) -> Self {
        Self::new(StatusCode::PAYLOAD_TOO_LARGE, "too_large", m)
    }
    pub fn quota(m: impl Into<String>) -> Self {
        Self::new(StatusCode::INSUFFICIENT_STORAGE, "quota", m)
    }
    pub fn rate_limited() -> Self {
        Self::new(StatusCode::TOO_MANY_REQUESTS, "rate_limited", "Zu viele Versuche. Bitte später erneut versuchen.")
    }
    pub fn not_implemented(m: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_IMPLEMENTED, "not_implemented", m)
    }
    pub fn upstream(m: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_GATEWAY, "upstream_error", m)
    }
    pub fn internal(m: impl std::fmt::Display) -> Self {
        eprintln!("Interner Fehler: {m}");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", "Interner Serverfehler.")
    }
    pub fn with_extra(mut self, extra: Value) -> Self {
        self.extra = Some(extra);
        self
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(e: rusqlite::Error) -> Self {
        Self::internal(e)
    }
}

impl From<std::io::Error> for ApiError {
    fn from(e: std::io::Error) -> Self {
        Self::internal(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut body = json!({"error": self.code, "message": self.message});
        if let (Some(Value::Object(extra)), Some(obj)) = (self.extra, body.as_object_mut()) {
            obj.extend(extra);
        }
        (self.status, axum::Json(body)).into_response()
    }
}
