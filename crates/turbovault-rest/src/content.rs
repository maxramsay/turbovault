//! Content-type negotiation for write endpoints.
//!
//! Two request body shapes are supported:
//!
//! * `application/json` — JSON object with at minimum a `"content"` field (and
//!   additional fields for patch operations).
//! * `text/markdown` or no Content-Type — raw body bytes treated as UTF-8 markdown.
//!
//! For patch requests arriving as `text/markdown`, the required `target_type`,
//! `target`, and `operation` fields are taken from query parameters instead.

use axum::{
    body::Bytes,
    http::HeaderMap,
};
use serde::Deserialize;

use crate::errors::ApiError;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Content extracted from a note write request (PUT / POST).
pub struct NoteContent {
    pub content: String,
}

/// All fields needed to perform a patch operation on a note.
pub struct PatchRequest {
    pub target_type: String,
    pub target: String,
    pub operation: String,
    pub content: String,
}

// ---------------------------------------------------------------------------
// Internal JSON shapes
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct JsonNoteBody {
    content: String,
}

#[derive(Deserialize)]
struct JsonPatchBody {
    target_type: String,
    target: String,
    operation: String,
    content: String,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Return `true` when the `Content-Type` header signals JSON.
fn is_json(headers: &HeaderMap) -> bool {
    headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|ct| ct.starts_with("application/json"))
        .unwrap_or(false)
}

/// Parse raw bytes as UTF-8, returning `ApiError::InvalidRequest` on failure.
fn bytes_to_string(body: Bytes) -> Result<String, ApiError> {
    String::from_utf8(body.to_vec()).map_err(|e| {
        ApiError::InvalidRequest(format!("Request body is not valid UTF-8: {}", e))
    })
}

// ---------------------------------------------------------------------------
// Extraction functions called from handlers
// ---------------------------------------------------------------------------

/// Extract a [`NoteContent`] from the raw request body and headers.
///
/// * JSON bodies must contain a `"content"` string field.
/// * Markdown / bare bodies are used verbatim.
pub fn extract_note_content(headers: &HeaderMap, body: Bytes) -> Result<NoteContent, ApiError> {
    if is_json(headers) {
        let parsed: JsonNoteBody = serde_json::from_slice(&body).map_err(|e| {
            ApiError::InvalidRequest(format!("Invalid JSON body: {}", e))
        })?;
        Ok(NoteContent {
            content: parsed.content,
        })
    } else {
        Ok(NoteContent {
            content: bytes_to_string(body)?,
        })
    }
}

/// Extract a [`PatchRequest`] from the raw request body, headers, and query params.
///
/// * JSON bodies must contain all four fields.
/// * Markdown / bare bodies use `target_type`, `target`, and `operation` from
///   the query string; the raw body becomes `content`.
pub fn extract_patch_request(
    headers: &HeaderMap,
    body: Bytes,
    query_target_type: Option<&str>,
    query_target: Option<&str>,
    query_operation: Option<&str>,
) -> Result<PatchRequest, ApiError> {
    if is_json(headers) {
        let parsed: JsonPatchBody = serde_json::from_slice(&body).map_err(|e| {
            ApiError::InvalidRequest(format!("Invalid JSON patch body: {}", e))
        })?;
        Ok(PatchRequest {
            target_type: parsed.target_type,
            target: parsed.target,
            operation: parsed.operation,
            content: parsed.content,
        })
    } else {
        let target_type = query_target_type
            .ok_or_else(|| ApiError::InvalidRequest("Missing query param: target_type".into()))?
            .to_owned();
        let target = query_target
            .ok_or_else(|| ApiError::InvalidRequest("Missing query param: target".into()))?
            .to_owned();
        let operation = query_operation
            .ok_or_else(|| ApiError::InvalidRequest("Missing query param: operation".into()))?
            .to_owned();

        Ok(PatchRequest {
            target_type,
            target,
            operation,
            content: bytes_to_string(body)?,
        })
    }
}

