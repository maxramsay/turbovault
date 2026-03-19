use axum::{
    extract::State,
    http::HeaderMap,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use turbovault_tools::file_tools::FileTools;

use crate::{errors::ApiError, response::ApiResponse, state::AppState, vault_resolver::resolve_vault};

#[derive(Deserialize)]
pub struct BatchReadRequest {
    pub paths: Vec<String>,
}

#[derive(Serialize)]
pub struct BatchReadData {
    pub results: Vec<Value>,
}

pub async fn batch_read(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<BatchReadRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if body.paths.is_empty() {
        return Err(ApiError::InvalidRequest(
            "paths array must not be empty".into(),
        ));
    }

    if body.paths.len() > 50 {
        return Err(ApiError::InvalidRequest(
            "Maximum 50 paths per batch request".into(),
        ));
    }

    let (vault_name, manager) = resolve_vault(&state, &headers).await?;

    let tools = FileTools::new(manager);
    let mut results: Vec<Value> = Vec::with_capacity(body.paths.len());
    let mut success_count = 0usize;

    for path in &body.paths {
        match tools.read_file(path).await {
            Ok(content) => {
                let hash = format!("{:x}", Sha256::digest(content.as_bytes()));
                results.push(json!({
                    "path": path,
                    "content": content,
                    "hash": hash,
                }));
                success_count += 1;
            }
            Err(e) => {
                let msg = e.to_string();
                let error_code = if msg.contains("not found") || msg.contains("No such file") {
                    "NOT_FOUND"
                } else {
                    "READ_ERROR"
                };
                results.push(json!({
                    "path": path,
                    "error": error_code,
                }));
            }
        }
    }

    let response_body = ApiResponse::new(
        &vault_name,
        "batch_read",
        BatchReadData { results },
    )
    .with_count(success_count);

    Ok(Json(response_body))
}
