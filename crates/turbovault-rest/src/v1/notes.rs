use axum::{
    extract::{Path, State},
    http::{HeaderMap, HeaderValue},
    response::IntoResponse,
    Json,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use turbovault_tools::file_tools::FileTools;

use crate::{errors::ApiError, response::ApiResponse, state::AppState, vault_resolver::resolve_vault};

#[derive(Serialize)]
pub struct NoteData {
    pub path: String,
    pub content: String,
    pub hash: String,
}

pub async fn read_note(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(path): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let (vault_name, manager) = resolve_vault(&state, &headers).await?;

    let tools = FileTools::new(manager);
    let content = tools.read_file(&path).await.map_err(|e| {
        let msg = e.to_string();
        if msg.contains("not found") || msg.contains("No such file") {
            ApiError::NotFound(format!("Note not found: {}", path))
        } else {
            ApiError::Internal(format!("Failed to read note: {}", msg))
        }
    })?;

    // Compute SHA-256 hash
    let hash = format!("{:x}", Sha256::digest(content.as_bytes()));

    let response_body = ApiResponse::new(
        &vault_name,
        "read_note",
        NoteData {
            path: path.clone(),
            content,
            hash: hash.clone(),
        },
    );

    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        "ETag",
        HeaderValue::from_str(&hash).unwrap_or_else(|_| HeaderValue::from_static("")),
    );

    Ok((response_headers, Json(response_body)))
}
