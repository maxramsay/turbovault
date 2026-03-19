use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use std::time::UNIX_EPOCH;

use crate::{errors::ApiError, response::ApiResponse, state::AppState, vault_resolver::resolve_vault};

#[derive(Deserialize)]
pub struct ListParams {
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}

fn default_limit() -> usize {
    200
}

#[derive(Serialize)]
pub struct FileEntry {
    pub name: String,
    #[serde(rename = "type")]
    pub entry_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified_at: Option<u64>,
}

fn is_hidden_or_excluded(name: &str) -> bool {
    name.starts_with('.') || name == ".trash" || name == ".obsidian"
}

fn list_dir_entries(
    dir: &std::path::Path,
) -> Result<Vec<FileEntry>, ApiError> {
    let read = std::fs::read_dir(dir)
        .map_err(|e| ApiError::NotFound(format!("Directory not found: {}", e)))?;

    let mut dirs: Vec<FileEntry> = Vec::new();
    let mut files: Vec<FileEntry> = Vec::new();

    for entry in read {
        let entry = entry.map_err(|e| ApiError::Internal(format!("Failed to read entry: {}", e)))?;
        let name = entry.file_name().to_string_lossy().into_owned();

        if is_hidden_or_excluded(&name) {
            continue;
        }

        let metadata = entry
            .metadata()
            .map_err(|e| ApiError::Internal(format!("Failed to read metadata: {}", e)))?;

        let modified_at = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs());

        if metadata.is_dir() {
            dirs.push(FileEntry {
                name,
                entry_type: "directory".to_string(),
                size_bytes: None,
                modified_at,
            });
        } else {
            files.push(FileEntry {
                name,
                entry_type: "file".to_string(),
                size_bytes: Some(metadata.len()),
                modified_at,
            });
        }
    }

    dirs.sort_by(|a, b| a.name.cmp(&b.name));
    files.sort_by(|a, b| a.name.cmp(&b.name));
    dirs.extend(files);

    Ok(dirs)
}

pub async fn list_root(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<ListParams>,
) -> Result<impl IntoResponse, ApiError> {
    let (vault_name, manager) = resolve_vault(&state, &headers).await?;
    let vault_path = manager.vault_path().clone();

    let all_entries = list_dir_entries(&vault_path)?;
    let total = all_entries.len();
    let has_more = params.offset + params.limit < total;

    let entries: Vec<FileEntry> = all_entries
        .into_iter()
        .skip(params.offset)
        .take(params.limit)
        .collect();

    let count = entries.len();

    let response = ApiResponse::new(&vault_name, "list_files", entries)
        .with_count(count)
        .with_has_more(has_more);

    Ok(Json(response))
}

pub async fn list_dir(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(path): Path<String>,
    Query(params): Query<ListParams>,
) -> Result<impl IntoResponse, ApiError> {
    let (vault_name, manager) = resolve_vault(&state, &headers).await?;
    let vault_path = manager.vault_path().clone();

    // Reject path traversal attempts
    let clean_path = path.trim_start_matches('/');
    if clean_path.contains("..") {
        return Err(ApiError::InvalidPath("Path traversal not allowed".into()));
    }

    let dir = vault_path.join(clean_path);

    // Ensure the resolved path is still under vault root
    let canonical_vault = vault_path
        .canonicalize()
        .map_err(|e| ApiError::Internal(format!("Failed to canonicalize vault path: {}", e)))?;
    let canonical_dir = dir
        .canonicalize()
        .map_err(|_| ApiError::NotFound(format!("Directory not found: {}", clean_path)))?;
    if !canonical_dir.starts_with(&canonical_vault) {
        return Err(ApiError::Forbidden("Path escapes vault root".into()));
    }

    if !canonical_dir.is_dir() {
        return Err(ApiError::NotFound(format!("Not a directory: {}", clean_path)));
    }

    let all_entries = list_dir_entries(&canonical_dir)?;
    let total = all_entries.len();
    let has_more = params.offset + params.limit < total;

    let entries: Vec<FileEntry> = all_entries
        .into_iter()
        .skip(params.offset)
        .take(params.limit)
        .collect();

    let count = entries.len();

    let response = ApiResponse::new(&vault_name, "list_files", entries)
        .with_count(count)
        .with_has_more(has_more);

    Ok(Json(response))
}
