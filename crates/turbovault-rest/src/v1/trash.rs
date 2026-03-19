//! Trash lifecycle endpoints: list, restore, request-purge.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::Serialize;

use crate::{
    errors::ApiError,
    pagination::{paginate, PaginationParams},
    response::ApiResponse,
    state::AppState,
    trash_manifest::TrashManifest,
    vault_resolver::resolve_vault,
};

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone)]
pub struct TrashListEntry {
    pub original_path: String,
    pub trash_path: String,
    pub deleted_at: String,
    pub orphaned_links: Vec<String>,
    pub permanent_delete_requested: Option<String>,
}

#[derive(Serialize)]
pub struct RestoreData {
    pub restored_to: String,
    pub previously_orphaned_links: Vec<String>,
}

#[derive(Serialize)]
pub struct PurgeRequestData {
    pub status: String,
    pub message: String,
    pub path: String,
    pub requested_at: String,
}

// ---------------------------------------------------------------------------
// GET /v1/trash
// ---------------------------------------------------------------------------

pub async fn list_trash(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(pagination): Query<PaginationParams>,
) -> Result<impl IntoResponse, ApiError> {
    let (vault_name, manager) = resolve_vault(&state, &headers).await?;
    let vault_path = manager.vault_path();

    let manifest = TrashManifest::load(vault_path)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to load trash manifest: {}", e)))?;

    let items: Vec<TrashListEntry> = manifest
        .entries
        .iter()
        .map(|e| TrashListEntry {
            original_path: e.original_path.clone(),
            trash_path: e.trash_path.clone(),
            deleted_at: e.deleted_at.clone(),
            orphaned_links: e.orphaned_links.clone(),
            permanent_delete_requested: e.permanent_delete_requested.clone(),
        })
        .collect();

    let (page, total, has_more) = paginate(items, &pagination);

    let response = ApiResponse::new(&vault_name, "list_trash", page)
        .with_count(total)
        .with_has_more(has_more);

    Ok(Json(response))
}

// ---------------------------------------------------------------------------
// POST /v1/restore/{*path}
// ---------------------------------------------------------------------------

pub async fn restore(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(trash_path): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let (vault_name, manager) = resolve_vault(&state, &headers).await?;
    let vault_path = manager.vault_path();

    let mut manifest = TrashManifest::load(vault_path)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to load trash manifest: {}", e)))?;

    // Find and remove entry
    let entry = manifest.remove_entry(&trash_path).ok_or_else(|| {
        ApiError::NotFound(format!("Trash entry not found: {}", trash_path))
    })?;

    let trash_file = vault_path.join(".trash").join(&entry.trash_path);
    let restore_target = vault_path.join(&entry.original_path);

    if !trash_file.exists() {
        return Err(ApiError::NotFound(format!(
            "Trash file missing from disk: {}",
            trash_path
        )));
    }

    // Create parent directories if needed
    if let Some(parent) = restore_target.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| ApiError::Internal(format!("Failed to create directories: {}", e)))?;
    }

    // Move file back
    tokio::fs::rename(&trash_file, &restore_target)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to restore file: {}", e)))?;

    // Save updated manifest
    manifest
        .save(vault_path)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to save manifest: {}", e)))?;

    let response = ApiResponse::new(
        &vault_name,
        "restore",
        RestoreData {
            restored_to: entry.original_path.clone(),
            previously_orphaned_links: entry.orphaned_links.clone(),
        },
    );

    Ok(Json(response))
}

// ---------------------------------------------------------------------------
// POST /v1/request-purge/{*path}
// ---------------------------------------------------------------------------

pub async fn request_purge(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(trash_path): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let (vault_name, manager) = resolve_vault(&state, &headers).await?;
    let vault_path = manager.vault_path();

    let mut manifest = TrashManifest::load(vault_path)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to load trash manifest: {}", e)))?;

    // Verify entry exists
    if manifest.find_entry(&trash_path).is_none() {
        return Err(ApiError::NotFound(format!(
            "Trash entry not found: {}",
            trash_path
        )));
    }

    if !manifest.mark_purge_requested(&trash_path) {
        return Err(ApiError::Internal(
            "Failed to mark purge request".into(),
        ));
    }

    // Re-read the entry for the response
    let entry = manifest.find_entry(&trash_path).unwrap();
    let requested_at = entry.permanent_delete_requested.clone().unwrap();
    let original_path = entry.original_path.clone();

    log::info!(
        "Permanent delete requested for: {}",
        original_path
    );

    manifest
        .save(vault_path)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to save manifest: {}", e)))?;

    let response = ApiResponse::new(
        &vault_name,
        "request_purge",
        PurgeRequestData {
            status: "pending".to_string(),
            message: "Permanent deletion requested. Awaiting curator processing.".to_string(),
            path: trash_path,
            requested_at,
        },
    );

    // Return 202 Accepted
    Ok((StatusCode::ACCEPTED, Json(response)))
}
