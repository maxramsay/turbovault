use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Vault not found: {0}")]
    VaultNotFound(String),

    #[error("Invalid path: {0}")]
    InvalidPath(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    #[error("Unauthorized")]
    Unauthorized,

    #[error("Hash mismatch")]
    HashMismatch,

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl ApiError {
    fn status_code(&self) -> StatusCode {
        match self {
            ApiError::NotFound(_) => StatusCode::NOT_FOUND,
            ApiError::VaultNotFound(_) => StatusCode::NOT_FOUND,
            ApiError::InvalidPath(_) => StatusCode::BAD_REQUEST,
            ApiError::Forbidden(_) => StatusCode::FORBIDDEN,
            ApiError::InvalidRequest(_) => StatusCode::BAD_REQUEST,
            ApiError::Unauthorized => StatusCode::UNAUTHORIZED,
            ApiError::HashMismatch => StatusCode::CONFLICT,
            ApiError::Conflict(_) => StatusCode::CONFLICT,
            ApiError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn error_code(&self) -> &'static str {
        match self {
            ApiError::NotFound(_) => "NOT_FOUND",
            ApiError::VaultNotFound(_) => "VAULT_NOT_FOUND",
            ApiError::InvalidPath(_) => "INVALID_PATH",
            ApiError::Forbidden(_) => "FORBIDDEN",
            ApiError::InvalidRequest(_) => "INVALID_REQUEST",
            ApiError::Unauthorized => "UNAUTHORIZED",
            ApiError::HashMismatch => "HASH_MISMATCH",
            ApiError::Conflict(_) => "CONFLICT",
            ApiError::Internal(_) => "INTERNAL_ERROR",
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        let code = self.error_code();
        let message = self.to_string();

        let body = Json(json!({
            "success": false,
            "error": {
                "code": code,
                "message": message
            }
        }));

        (status, body).into_response()
    }
}
