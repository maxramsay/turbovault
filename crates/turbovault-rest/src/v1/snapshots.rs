use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};

use turbovault_core::events::*;
use turbovault_core::snapshot::{RestoreMode, SnapshotCreateRequest, SnapshotSelection};
use turbovault_tools::snapshot_tools::{SnapshotError, SnapshotTools};

use crate::{errors::ApiError, state::AppState, vault_resolver::resolve_vault};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Map `SnapshotError` variants to appropriate `ApiError` responses.
fn map_snapshot_error(e: SnapshotError) -> ApiError {
    match e {
        SnapshotError::NoMatchingNotes => {
            ApiError::InvalidRequest("No notes matched the selection filter".into())
        }
        SnapshotError::NotFound(id) => ApiError::NotFound(format!("Snapshot not found: {}", id)),
        SnapshotError::ManifestNotFound => {
            ApiError::NotFound("Manifest not found in snapshot archive".into())
        }
        SnapshotError::Io(e) => ApiError::Internal(format!("I/O error: {}", e)),
        SnapshotError::Serialization(e) => {
            ApiError::Internal(format!("Serialization error: {}", e))
        }
        SnapshotError::PathError => ApiError::Internal("Path resolution error".into()),
    }
}

/// Return the default snapshot target directory from the environment or a fallback.
fn default_target() -> String {
    std::env::var("FC_SNAPSHOT_TARGET").unwrap_or_else(|_| "/tmp/vault-snapshots".to_string())
}

// ---------------------------------------------------------------------------
// POST /v1/snapshots — Create a snapshot
// ---------------------------------------------------------------------------

pub async fn create_snapshot(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<SnapshotCreateRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let (_vault_name, manager) = resolve_vault(&state, &headers).await?;
    let vault_path = manager.vault_path().clone();

    let selection = match &body.tags {
        Some(tags) if !tags.is_empty() => SnapshotSelection::Tags { tags: tags.clone() },
        _ => SnapshotSelection::All,
    };

    let target = body.target.clone().unwrap_or_else(default_target);
    let org_id = body.org_id.clone().unwrap_or_else(|| "default".to_string());

    let tools = SnapshotTools::new(&vault_path);
    let manifest = tools
        .create_snapshot(&selection, &target, &org_id, "rest-api")
        .await
        .map_err(map_snapshot_error)?;

    state
        .publisher
        .emit(
            "vault.snapshot.created",
            &SnapshotCreatedEvent {
                snapshot_id: manifest.snapshot_id.clone(),
                path: target.clone(),
                version: manifest.note_count as u64,
            },
        )
        .await;

    Ok((StatusCode::CREATED, Json(manifest)))
}

// ---------------------------------------------------------------------------
// GET /v1/snapshots — List snapshots
// ---------------------------------------------------------------------------

pub async fn list_snapshots(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let (_vault_name, manager) = resolve_vault(&state, &headers).await?;
    let vault_path = manager.vault_path().clone();

    let target = default_target();
    let tools = SnapshotTools::new(&vault_path);
    let manifests = tools
        .list_snapshots(&target)
        .await
        .map_err(map_snapshot_error)?;

    let count = manifests.len();

    state
        .publisher
        .emit(
            "vault.snapshot.listed",
            &SnapshotListedEvent {
                path: target,
                count,
            },
        )
        .await;

    Ok(Json(manifests))
}

// ---------------------------------------------------------------------------
// GET /v1/snapshots/{id} — Inspect a snapshot
// ---------------------------------------------------------------------------

pub async fn get_snapshot(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let (_vault_name, _manager) = resolve_vault(&state, &headers).await?;

    let target = default_target();
    let archive_path =
        std::path::PathBuf::from(&target).join(format!("{}.tar.gz", &id));

    if !archive_path.exists() {
        return Err(ApiError::NotFound(format!("Snapshot not found: {}", id)));
    }

    let manifest =
        SnapshotTools::read_manifest_from_archive(&archive_path).map_err(map_snapshot_error)?;

    Ok(Json(manifest))
}

// ---------------------------------------------------------------------------
// POST /v1/snapshots/{id}/restore — Restore a snapshot
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct RestoreBody {
    pub mode: RestoreMode,
}

pub async fn restore_snapshot(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<RestoreBody>,
) -> Result<impl IntoResponse, ApiError> {
    let (_vault_name, manager) = resolve_vault(&state, &headers).await?;
    let vault_path = manager.vault_path().clone();

    let target = default_target();
    let archive_path =
        std::path::PathBuf::from(&target).join(format!("{}.tar.gz", &id));

    if !archive_path.exists() {
        return Err(ApiError::NotFound(format!("Snapshot not found: {}", id)));
    }

    let tools = SnapshotTools::new(&vault_path);
    let manifest = tools
        .restore_snapshot(&archive_path, &body.mode)
        .await
        .map_err(map_snapshot_error)?;

    state
        .publisher
        .emit(
            "vault.snapshot.restored",
            &SnapshotRestoredEvent {
                snapshot_id: manifest.snapshot_id.clone(),
                path: target,
                restored_version: manifest.note_count as u64,
            },
        )
        .await;

    Ok(Json(manifest))
}

// ---------------------------------------------------------------------------
// DELETE /v1/snapshots/{id} — Delete a snapshot
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct DeleteResponse {
    deleted: String,
}

pub async fn delete_snapshot(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let (_vault_name, manager) = resolve_vault(&state, &headers).await?;
    let vault_path = manager.vault_path().clone();

    let target = default_target();
    let tools = SnapshotTools::new(&vault_path);
    tools
        .delete_snapshot(&target, &id)
        .await
        .map_err(map_snapshot_error)?;

    state
        .publisher
        .emit(
            "vault.snapshot.deleted",
            &SnapshotDeletedEvent {
                snapshot_id: id.clone(),
                path: target,
            },
        )
        .await;

    Ok(Json(DeleteResponse { deleted: id }))
}
