use axum::{
    extract::{Query, State},
    http::HeaderMap,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use turbovault_tools::search_engine::SearchEngine;

use crate::{errors::ApiError, response::ApiResponse, state::AppState, vault_resolver::resolve_vault};

#[derive(Deserialize)]
pub struct SearchParams {
    pub q: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}

fn default_limit() -> usize {
    50
}

#[derive(Serialize)]
pub struct SearchResultItem {
    pub path: String,
    pub title: String,
    pub score: f64,
    pub snippet: String,
}

pub async fn search(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<SearchParams>,
) -> Result<impl IntoResponse, ApiError> {
    let query = params.q.as_deref().unwrap_or("").trim().to_string();
    if query.is_empty() {
        return Err(ApiError::InvalidRequest(
            "Query parameter 'q' is required and must not be empty".into(),
        ));
    }

    let (vault_name, manager) = resolve_vault(&state, &headers).await?;

    let engine = SearchEngine::new(manager)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to initialize search engine: {}", e)))?;

    let all_results = engine
        .search(&query)
        .await
        .map_err(|e| ApiError::Internal(format!("Search failed: {}", e)))?;

    let total = all_results.len();
    let has_more = params.offset + params.limit < total;

    let results: Vec<SearchResultItem> = all_results
        .into_iter()
        .skip(params.offset)
        .take(params.limit)
        .map(|r| SearchResultItem {
            path: r.path,
            title: r.title,
            score: r.score,
            snippet: if r.snippet.is_empty() { r.preview } else { r.snippet },
        })
        .collect();

    let count = results.len();

    let response = ApiResponse::new(&vault_name, "search", results)
        .with_count(count)
        .with_has_more(has_more);

    Ok(Json(response))
}
