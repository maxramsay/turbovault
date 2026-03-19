use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue},
    response::IntoResponse,
    Json,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path as FsPath;
use std::time::{SystemTime, UNIX_EPOCH};
use turbovault_tools::file_tools::{FileTools, WriteMode};

use crate::{
    content, errors::ApiError, response::ApiResponse, state::{AppState, RestConfig},
    trash_manifest::{TrashEntry, TrashManifest},
    vault_resolver::resolve_vault,
};

/// Return `Err(ApiError::Forbidden)` if `path` starts with any protected prefix.
fn check_protected_path(path: &str, config: &RestConfig) -> Result<(), ApiError> {
    for protected in &config.protected_paths {
        if path.starts_with(protected.as_str()) {
            return Err(ApiError::Forbidden(format!("Path is protected: {}", protected)));
        }
    }
    Ok(())
}

/// Return `Err(ApiError::HashMismatch)` if an `If-Match` header is present and
/// does not match the SHA-256 of the file's current content.
///
/// Returns `Err(ApiError::NotFound)` if the header is present but the file does
/// not exist (can't validate a hash against nothing).
async fn check_if_match(
    headers: &HeaderMap,
    vault_path: &FsPath,
    note_path: &str,
) -> Result<(), ApiError> {
    if let Some(expected_hash_val) = headers.get("If-Match") {
        let expected_hash = expected_hash_val
            .to_str()
            .unwrap_or("")
            .trim_matches('"'); // strip optional surrounding quotes

        let current_content = std::fs::read_to_string(vault_path.join(note_path))
            .map_err(|_| ApiError::NotFound(note_path.to_string()))?;

        let current_hash = format!("{:x}", Sha256::digest(current_content.as_bytes()));

        if expected_hash != current_hash {
            return Err(ApiError::HashMismatch);
        }
    }
    Ok(())
}

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

#[derive(Serialize)]
pub struct PatchData {
    pub path: String,
    pub target_type: String,
    pub target: String,
    pub operation: String,
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
    check_protected_path(&path, &state.config)?;

    let (vault_name, manager) = resolve_vault(&state, &headers).await?;

    let vault_path = manager.vault_path().to_path_buf();
    check_if_match(&headers, &vault_path, &path).await?;

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

pub async fn patch_note(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(path): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    body: Bytes,
) -> Result<impl IntoResponse, ApiError> {
    check_protected_path(&path, &state.config)?;

    let (vault_name, manager) = resolve_vault(&state, &headers).await?;

    let vault_path = manager.vault_path().to_path_buf();
    check_if_match(&headers, &vault_path, &path).await?;

    let tools = FileTools::new(manager.clone());

    // PATCH requires the note to already exist
    let existing = tools.read_file(&path).await.map_err(|e| {
        let msg = e.to_string();
        if msg.contains("not found") || msg.contains("No such file") {
            ApiError::NotFound(format!("Note not found: {}", path))
        } else {
            ApiError::Internal(format!("Failed to read note: {}", msg))
        }
    })?;

    let patch_req = content::extract_patch_request(
        &headers,
        body,
        query.get("target_type").map(|s| s.as_str()),
        query.get("target").map(|s| s.as_str()),
        query.get("operation").map(|s| s.as_str()),
    )?;

    let target_type = patch_req.target_type;
    let target = patch_req.target;
    let content_to_insert = patch_req.content;

    let op = match patch_req.operation.to_lowercase().as_str() {
        "append" | "prepend" | "replace" => patch_req.operation.to_lowercase(),
        _ => {
            return Err(ApiError::InvalidRequest(format!(
                "Invalid operation '{}'. Valid: append, prepend, replace",
                patch_req.operation
            )))
        }
    };

    let new_content = match target_type.to_lowercase().as_str() {
        "heading" => {
            let lines: Vec<&str> = existing.lines().collect();
            let target_heading = target.trim().trim_start_matches('#').trim();

            let mut heading_line_idx = None;
            let mut heading_level = 0;
            let mut available_headings: Vec<String> = Vec::new();
            let mut in_code_block = false;

            for (i, line) in lines.iter().enumerate() {
                let trimmed = line.trim();
                if trimmed.starts_with("```") {
                    in_code_block = !in_code_block;
                    continue;
                }
                if in_code_block {
                    continue;
                }
                if trimmed.starts_with('#') {
                    let level = trimmed.chars().take_while(|c| *c == '#').count();
                    let heading_text = trimmed.trim_start_matches('#').trim();
                    available_headings.push(trimmed.to_string());
                    if heading_text.eq_ignore_ascii_case(target_heading) && heading_line_idx.is_none() {
                        heading_line_idx = Some(i);
                        heading_level = level;
                    }
                }
            }

            let heading_idx = match heading_line_idx {
                Some(idx) => idx,
                None => {
                    return Err(ApiError::InvalidRequest(format!(
                        "Heading '{}' not found. Available headings: {}",
                        target,
                        available_headings.join(", ")
                    )));
                }
            };

            let mut section_end = lines.len();
            let mut in_code_block_end = false;
            for i in (heading_idx + 1)..lines.len() {
                let trimmed = lines[i].trim();
                if trimmed.starts_with("```") {
                    in_code_block_end = !in_code_block_end;
                    continue;
                }
                if in_code_block_end {
                    continue;
                }
                if trimmed.starts_with('#') {
                    let level = trimmed.chars().take_while(|c| *c == '#').count();
                    if level <= heading_level {
                        section_end = i;
                        break;
                    }
                }
            }

            let mut new_lines: Vec<String> = Vec::with_capacity(lines.len() + 10);
            match op.as_str() {
                "prepend" => {
                    new_lines.extend(lines[..=heading_idx].iter().map(|s| s.to_string()));
                    new_lines.push(String::new());
                    for line in content_to_insert.lines() {
                        new_lines.push(line.to_string());
                    }
                    new_lines.extend(lines[heading_idx + 1..].iter().map(|s| s.to_string()));
                }
                "append" => {
                    new_lines.extend(lines[..section_end].iter().map(|s| s.to_string()));
                    if !new_lines.last().map_or(true, |l| l.trim().is_empty()) {
                        new_lines.push(String::new());
                    }
                    for line in content_to_insert.lines() {
                        new_lines.push(line.to_string());
                    }
                    if section_end < lines.len() {
                        new_lines.push(String::new());
                    }
                    new_lines.extend(lines[section_end..].iter().map(|s| s.to_string()));
                }
                "replace" => {
                    new_lines.extend(lines[..=heading_idx].iter().map(|s| s.to_string()));
                    new_lines.push(String::new());
                    for line in content_to_insert.lines() {
                        new_lines.push(line.to_string());
                    }
                    if section_end < lines.len() {
                        new_lines.push(String::new());
                    }
                    new_lines.extend(lines[section_end..].iter().map(|s| s.to_string()));
                }
                _ => unreachable!(),
            }
            new_lines.join("\n")
        }
        "block" => {
            let block_ref = if target.starts_with('^') {
                target.clone()
            } else {
                format!("^{}", target)
            };
            let lines: Vec<&str> = existing.lines().collect();
            let block_idx = lines.iter().position(|line| line.contains(&block_ref));
            match block_idx {
                Some(idx) => {
                    let mut new_lines: Vec<String> = Vec::with_capacity(lines.len() + 5);
                    match op.as_str() {
                        "prepend" => {
                            new_lines.extend(lines[..idx].iter().map(|s| s.to_string()));
                            for line in content_to_insert.lines() {
                                new_lines.push(line.to_string());
                            }
                            new_lines.extend(lines[idx..].iter().map(|s| s.to_string()));
                        }
                        "append" => {
                            new_lines.extend(lines[..=idx].iter().map(|s| s.to_string()));
                            for line in content_to_insert.lines() {
                                new_lines.push(line.to_string());
                            }
                            new_lines.extend(lines[idx + 1..].iter().map(|s| s.to_string()));
                        }
                        "replace" => {
                            new_lines.extend(lines[..idx].iter().map(|s| s.to_string()));
                            for line in content_to_insert.lines() {
                                new_lines.push(line.to_string());
                            }
                            new_lines.extend(lines[idx + 1..].iter().map(|s| s.to_string()));
                        }
                        _ => unreachable!(),
                    }
                    new_lines.join("\n")
                }
                None => {
                    return Err(ApiError::InvalidRequest(format!(
                        "Block reference '{}' not found in {}",
                        block_ref, path
                    )))
                }
            }
        }
        "frontmatter" => {
            if !existing.starts_with("---") {
                return Err(ApiError::InvalidRequest("Note has no frontmatter".into()));
            }
            let fm_end = existing[3..].find("\n---").map(|i| i + 3 + 4);
            match fm_end {
                Some(end) => {
                    let fm_section = &existing[..end];
                    let body_str = &existing[end..];
                    let mut fm_lines: Vec<String> =
                        fm_section.lines().map(|s| s.to_string()).collect();
                    let key_prefix = format!("{}:", target);
                    let mut found = false;
                    for line in fm_lines.iter_mut() {
                        if line.trim_start().starts_with(&key_prefix) {
                            match op.as_str() {
                                "replace" => *line = format!("{}: {}", target, content_to_insert),
                                "append" => *line = format!("{} {}", line, content_to_insert),
                                "prepend" => {
                                    let val_start = line.find(':').unwrap() + 1;
                                    let existing_val = line[val_start..].trim();
                                    *line = format!(
                                        "{}: {} {}",
                                        target, content_to_insert, existing_val
                                    );
                                }
                                _ => unreachable!(),
                            }
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        let last = fm_lines.len() - 1;
                        fm_lines
                            .insert(last, format!("{}: {}", target, content_to_insert));
                    }
                    format!("{}{}", fm_lines.join("\n"), body_str)
                }
                None => {
                    return Err(ApiError::InvalidRequest(
                        "Malformed frontmatter (missing closing ---)".into(),
                    ))
                }
            }
        }
        _ => {
            return Err(ApiError::InvalidRequest(format!(
                "Invalid target_type '{}'. Valid: heading, block, frontmatter",
                target_type
            )))
        }
    };

    tools
        .write_file_with_mode(&path, &new_content, WriteMode::Overwrite)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to write patched note: {}", e)))?;

    let response_body = ApiResponse::new(
        &vault_name,
        "patch_note",
        PatchData {
            path: path.clone(),
            target_type,
            target,
            operation: op,
            status: "patched".to_string(),
        },
    );

    Ok(Json(response_body))
}

pub async fn append_note(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(path): Path<String>,
    body: Bytes,
) -> Result<impl IntoResponse, ApiError> {
    check_protected_path(&path, &state.config)?;

    let (vault_name, manager) = resolve_vault(&state, &headers).await?;

    let vault_path = manager.vault_path().to_path_buf();
    check_if_match(&headers, &vault_path, &path).await?;

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

#[derive(Serialize)]
pub struct DeleteData {
    pub original_path: String,
    pub moved_to: String,
    pub orphaned_links: Vec<String>,
    pub restorable: bool,
}

pub async fn delete_note(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(path): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    check_protected_path(&path, &state.config)?;

    let (vault_name, manager) = resolve_vault(&state, &headers).await?;
    let vault_path = manager.vault_path();

    check_if_match(&headers, vault_path, &path).await?;

    let full_path = vault_path.join(&path);
    if !full_path.exists() {
        return Err(ApiError::NotFound(format!("Note not found: {}", path)));
    }

    // TODO: Compute orphaned links by scanning for backlinks.
    // Graph initialization per-request is expensive; returning empty list for now.
    let orphaned_links: Vec<String> = Vec::new();

    // Build trash path: {original-path}.{unix_timestamp}
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let trash_relative = format!("{}.{}", path, timestamp);

    let trash_dest = vault_path.join(".trash").join(&trash_relative);

    // Create parent directories in .trash/ as needed
    if let Some(parent) = trash_dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| ApiError::Internal(format!("Failed to create trash directory: {}", e)))?;
    }

    // Move file to trash
    tokio::fs::rename(&full_path, &trash_dest)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to move file to trash: {}", e)))?;

    // Update manifest
    let mut manifest = TrashManifest::load(vault_path)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to load trash manifest: {}", e)))?;

    let entry = TrashEntry {
        original_path: path.clone(),
        trash_path: trash_relative.clone(),
        deleted_at: chrono::Utc::now().to_rfc3339(),
        orphaned_links: orphaned_links.clone(),
        permanent_delete_requested: None,
    };

    manifest
        .add_entry(entry, vault_path)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to save trash manifest: {}", e)))?;

    let response = ApiResponse::new(
        &vault_name,
        "delete_note",
        DeleteData {
            original_path: path,
            moved_to: trash_relative,
            orphaned_links,
            restorable: true,
        },
    );

    Ok(Json(response))
}
