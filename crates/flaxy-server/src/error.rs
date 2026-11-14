use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

/// Erro da API: vira `{ "detail": "..." }` com o status certo.
pub struct ApiError(pub StatusCode, pub String);

impl ApiError {
    pub fn bad(msg: impl Into<String>) -> Self {
        Self(StatusCode::BAD_REQUEST, msg.into())
    }
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self(StatusCode::NOT_FOUND, msg.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "detail": self.1 }))).into_response()
    }
}

impl From<flaxy_core::Error> for ApiError {
    fn from(e: flaxy_core::Error) -> Self {
        use flaxy_core::Error as E;
        let code = match &e {
            E::NotFound(_) => StatusCode::NOT_FOUND,
            E::Invalid(_) => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        if code == StatusCode::INTERNAL_SERVER_ERROR {
            tracing::error!("{e}");
        }
        Self(code, e.to_string())
    }
}

impl From<std::io::Error> for ApiError {
    fn from(e: std::io::Error) -> Self {
        tracing::error!("io: {e}");
        Self(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
