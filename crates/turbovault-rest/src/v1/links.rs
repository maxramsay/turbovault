use axum::{
    extract::{Path, State},
    http::HeaderMap,
    response::IntoResponse,
    Json,
};
use serde::Serialize;

use crate::{errors::ApiError, response::ApiResponse, state::AppState, vault_resolver::resolve_vault};

#[derive(Serialize)]
pub struct LinksData {
    pub path: String,
    pub links: Vec<String>,
    pub count: usize,
}

pub async fn backlinks(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(path): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let (vault_name, manager) = resolve_vault(&state, &headers).await?;
    let vault_root = manager.vault_path().clone();

    let note_path = std::path::Path::new(&path);
    let backlinks = manager
        .get_backlinks(note_path)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get backlinks: {}", e)))?;

    let links: Vec<String> = backlinks
        .into_iter()
        .filter_map(|p| {
            p.strip_prefix(&vault_root)
                .ok()
                .map(|rel| rel.to_string_lossy().into_owned())
        })
        .collect();

    let count = links.len();
    let response = ApiResponse::new(&vault_name, "backlinks", LinksData { path, links, count })
        .with_count(count);

    Ok(Json(response))
}

pub async fn forward_links(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(path): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let (vault_name, manager) = resolve_vault(&state, &headers).await?;
    let vault_root = manager.vault_path().clone();

    let note_path = std::path::Path::new(&path);
    let forward = manager
        .get_forward_links(note_path)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get forward links: {}", e)))?;

    let links: Vec<String> = forward
        .into_iter()
        .filter_map(|p| {
            p.strip_prefix(&vault_root)
                .ok()
                .map(|rel| rel.to_string_lossy().into_owned())
        })
        .collect();

    let count = links.len();
    let response = ApiResponse::new(&vault_name, "forward_links", LinksData { path, links, count })
        .with_count(count);

    Ok(Json(response))
}
