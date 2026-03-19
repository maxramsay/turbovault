use axum::{
    extract::{Path, State},
    http::HeaderMap,
    response::IntoResponse,
};
use serde::Serialize;
use std::time::UNIX_EPOCH;
use turbovault_tools::file_tools::FileTools;

use crate::{errors::ApiError, response::ApiResponse, state::AppState, vault_resolver::resolve_vault};

#[derive(Serialize)]
pub struct NoteInfoData {
    pub path: String,
    pub size_bytes: u64,
    pub modified_at: Option<String>,
    pub has_frontmatter: bool,
}

pub async fn get_info(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(path): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let (vault_name, manager) = resolve_vault(&state, &headers).await?;

    // Resolve the full filesystem path using VaultManager (provides path traversal protection)
    let full_path = manager
        .resolve_path(&std::path::PathBuf::from(&path))
        .map_err(|e| ApiError::InvalidPath(e.to_string()))?;

    let meta = std::fs::metadata(&full_path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            ApiError::NotFound(format!("Note not found: {}", path))
        } else {
            ApiError::Internal(format!("Failed to read metadata: {}", e))
        }
    })?;

    let size_bytes = meta.len();

    let modified_at = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .and_then(|d| {
            chrono::DateTime::from_timestamp(d.as_secs() as i64, d.subsec_nanos())
                .map(|dt| dt.to_rfc3339())
        });

    // Check for frontmatter by reading the first few bytes
    let has_frontmatter = if size_bytes >= 4 {
        let mut buf = [0u8; 5];
        match std::fs::File::open(&full_path)
            .and_then(|mut f| {
                use std::io::Read;
                let n = f.read(&mut buf)?;
                Ok(n)
            }) {
            Ok(n) => {
                let slice = &buf[..n];
                slice.starts_with(b"---\n") || slice.starts_with(b"---\r\n")
            }
            Err(_) => false,
        }
    } else {
        false
    };

    // Use FileTools only to validate file is readable (path already resolved above)
    // We use the manager directly via resolve_path above, so no need to call FileTools here.
    let _ = FileTools::new(manager); // ensure the type is used (compile check)

    Ok(ApiResponse::new(
        &vault_name,
        "notes_info",
        NoteInfoData {
            path,
            size_bytes,
            modified_at,
            has_frontmatter,
        },
    ))
}
