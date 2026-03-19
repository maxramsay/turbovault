//! Bearer token authentication middleware for the REST API.
//!
//! When `RestConfig.api_token` is `Some(token)`, every request must carry
//! `Authorization: Bearer <token>`. If the header is absent or the token does
//! not match, the middleware returns 401 immediately without forwarding to the
//! inner handler.
//!
//! When `api_token` is `None` the middleware is a no-op (LAN trust mode).

use axum::{extract::State, middleware::Next, response::Response};

use crate::{errors::ApiError, state::AppState};

/// Axum middleware that enforces Bearer token authentication.
pub async fn auth_middleware(
    State(state): State<AppState>,
    request: axum::extract::Request,
    next: Next,
) -> Result<Response, ApiError> {
    if let Some(ref expected_token) = state.config.api_token {
        let auth_header = request
            .headers()
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok());

        let valid = match auth_header {
            Some(header) => {
                if let Some(token) = header.strip_prefix("Bearer ") {
                    token == expected_token
                } else {
                    false
                }
            }
            None => false,
        };

        if !valid {
            return Err(ApiError::Unauthorized);
        }
    }

    Ok(next.run(request).await)
}
