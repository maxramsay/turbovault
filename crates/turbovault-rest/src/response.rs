use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;

#[derive(Serialize)]
pub struct ApiResponse<T: Serialize> {
    pub vault: String,
    pub operation: String,
    pub success: bool,
    pub data: T,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_more: Option<bool>,
    pub took_ms: u64,
}

impl<T: Serialize> ApiResponse<T> {
    pub fn new(vault: impl Into<String>, operation: impl Into<String>, data: T) -> Self {
        Self {
            vault: vault.into(),
            operation: operation.into(),
            success: true,
            data,
            count: None,
            has_more: None,
            took_ms: 0,
        }
    }

    pub fn with_count(mut self, n: usize) -> Self {
        self.count = Some(n);
        self
    }

    pub fn with_has_more(mut self, b: bool) -> Self {
        self.has_more = Some(b);
        self
    }

    pub fn with_duration(mut self, ms: u64) -> Self {
        self.took_ms = ms;
        self
    }
}

impl<T: Serialize> IntoResponse for ApiResponse<T> {
    fn into_response(self) -> Response {
        (StatusCode::OK, Json(self)).into_response()
    }
}
