use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue},
    response::IntoResponse,
    Json,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use turbovault_tools::file_tools::{FileTools, WriteMode};

use crate::{content, errors::ApiError, response::ApiResponse, state::AppState, vault_resolver::resolve_vault};

#[derive(Serialize)]
pub struct NoteData {
    pub path: String,
    pub content: String,
    pub hash: String,
}

#[derive(Serialize)]
pub struct WriteData {
    pub path: String,
    pub hash: String,
    pub status: String,
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

pub async fn create_note(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(path): Path<String>,
    body: Bytes,
) -> Result<impl IntoResponse, ApiError> {
    let (vault_name, manager) = resolve_vault(&state, &headers).await?;

    let note_content = content::extract_note_content(&headers, body)?;

    let tools = FileTools::new(manager.clone());

    // Determine if the file already exists to set the status string.
    let file_exists = tools.read_file(&path).await.is_ok();

    tools
        .write_file_with_mode(&path, &note_content.content, WriteMode::Overwrite)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to write note: {}", e)))?;

    let hash = format!("{:x}", Sha256::digest(note_content.content.as_bytes()));
    let status = if file_exists { "overwritten" } else { "created" }.to_string();

    let response_body = ApiResponse::new(
        &vault_name,
        "create_note",
        WriteData {
            path: path.clone(),
            hash: hash.clone(),
            status,
        },
    );

    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        "ETag",
        HeaderValue::from_str(&hash).unwrap_or_else(|_| HeaderValue::from_static("")),
    );

    Ok((response_headers, Json(response_body)))
}

pub async fn append_note(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(path): Path<String>,
    body: Bytes,
) -> Result<impl IntoResponse, ApiError> {
    let (vault_name, manager) = resolve_vault(&state, &headers).await?;

    let tools = FileTools::new(manager.clone());

    // POST is append-only; refuse if the file does not already exist.
    tools.read_file(&path).await.map_err(|e| {
        let msg = e.to_string();
        if msg.contains("not found") || msg.contains("No such file") {
            ApiError::NotFound(format!("Note not found: {}", path))
        } else {
            ApiError::Internal(format!("Failed to check note existence: {}", msg))
        }
    })?;

    let note_content = content::extract_note_content(&headers, body)?;

    tools
        .write_file_with_mode(&path, &note_content.content, WriteMode::Append)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to append to note: {}", e)))?;

    // Read back the full content to compute the post-append hash.
    let full_content = tools
        .read_file(&path)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to read note after append: {}", e)))?;

    let hash = format!("{:x}", Sha256::digest(full_content.as_bytes()));

    let response_body = ApiResponse::new(
        &vault_name,
        "append_note",
        WriteData {
            path: path.clone(),
            hash: hash.clone(),
            status: "appended".to_string(),
        },
    );

    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        "ETag",
        HeaderValue::from_str(&hash).unwrap_or_else(|_| HeaderValue::from_static("")),
    );

    Ok((response_headers, Json(response_body)))
}
