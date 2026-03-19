use axum::{
    extract::{Query, State},
    http::HeaderMap,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

use crate::{errors::ApiError, response::ApiResponse, state::AppState, vault_resolver::resolve_vault};

#[derive(Deserialize)]
pub struct RecentParams {
    #[serde(default = "default_days")]
    pub days: u64,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}

fn default_days() -> u64 {
    7
}

fn default_limit() -> usize {
    50
}

#[derive(Serialize)]
pub struct RecentEntry {
    pub path: String,
    pub modified_at: u64,
    pub size_bytes: u64,
}

fn is_excluded_dir(name: &str) -> bool {
    name.starts_with('.') || name == ".trash" || name == ".obsidian"
}

pub async fn get_recent(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<RecentParams>,
) -> Result<impl IntoResponse, ApiError> {
    let (vault_name, manager) = resolve_vault(&state, &headers).await?;
    let vault_path = manager.vault_path().clone();

    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(params.days * 86400))
        .unwrap_or(UNIX_EPOCH);

    let mut entries: Vec<RecentEntry> = Vec::new();

    for entry in WalkDir::new(&vault_path)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            // Exclude hidden directories and special obsidian dirs
            if e.file_type().is_dir() {
                if let Some(name) = e.file_name().to_str() {
                    if is_excluded_dir(name) && e.depth() > 0 {
                        return false;
                    }
                }
            }
            true
        })
    {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        if !entry.file_type().is_file() {
            continue;
        }

        // Only .md files
        if entry.path().extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }

        // Skip hidden files
        if let Some(name) = entry.file_name().to_str() {
            if name.starts_with('.') {
                continue;
            }
        }

        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };

        let modified = match metadata.modified() {
            Ok(t) => t,
            Err(_) => continue,
        };

        if modified < cutoff {
            continue;
        }

        let modified_secs = modified
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Compute path relative to vault root
        let rel_path = entry
            .path()
            .strip_prefix(&vault_path)
            .unwrap_or(entry.path())
            .to_string_lossy()
            .into_owned();

        entries.push(RecentEntry {
            path: rel_path,
            modified_at: modified_secs,
            size_bytes: metadata.len(),
        });
    }

    // Sort newest first
    entries.sort_by(|a, b| b.modified_at.cmp(&a.modified_at));

    let total = entries.len();
    let has_more = params.offset + params.limit < total;

    let page: Vec<RecentEntry> = entries
        .into_iter()
        .skip(params.offset)
        .take(params.limit)
        .collect();

    let count = page.len();

    let response = ApiResponse::new(&vault_name, "recent_changes", page)
        .with_count(count)
        .with_has_more(has_more);

    Ok(Json(response))
}
