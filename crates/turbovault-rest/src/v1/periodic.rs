use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    response::IntoResponse,
    Json,
};
use chrono::Datelike;
use serde::{Deserialize, Serialize};
use turbovault_tools::file_tools::FileTools;

use crate::{errors::ApiError, response::ApiResponse, state::AppState, vault_resolver::resolve_vault};

#[derive(Deserialize)]
pub struct PeriodicParams {
    pub date: Option<String>,
}

#[derive(Serialize)]
pub struct PeriodicData {
    pub path: String,
    pub period: String,
    pub date: String,
    pub exists: bool,
    pub content: Option<String>,
}

pub async fn get_periodic(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(period): Path<String>,
    Query(params): Query<PeriodicParams>,
) -> Result<impl IntoResponse, ApiError> {
    let (vault_name, manager) = resolve_vault(&state, &headers).await?;

    let target_date = if let Some(date_str) = &params.date {
        chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d").map_err(|e| {
            ApiError::InvalidRequest(format!(
                "Invalid date format (expected YYYY-MM-DD): {}",
                e
            ))
        })?
    } else {
        chrono::Local::now().date_naive()
    };

    let path = match period.to_lowercase().as_str() {
        "daily" => format!("Daily/{}.md", target_date.format("%Y-%m-%d")),
        "weekly" => {
            let iso_week = target_date.iso_week();
            format!("Daily/{}-W{:02}.md", iso_week.year(), iso_week.week())
        }
        "monthly" => format!("Daily/{}.md", target_date.format("%Y-%m")),
        "quarterly" => {
            let quarter = (target_date.month() - 1) / 3 + 1;
            format!("Daily/{}-Q{}.md", target_date.year(), quarter)
        }
        "yearly" => format!("Daily/{}.md", target_date.format("%Y")),
        _ => {
            return Err(ApiError::InvalidRequest(format!(
                "Invalid period '{}'. Valid: daily, weekly, monthly, quarterly, yearly",
                period
            )))
        }
    };

    let date_str = target_date.format("%Y-%m-%d").to_string();

    let tools = FileTools::new(manager);
    match tools.read_file(&path).await {
        Ok(content) => {
            let response = ApiResponse::new(
                &vault_name,
                "get_periodic_note",
                PeriodicData {
                    path,
                    period,
                    date: date_str,
                    exists: true,
                    content: Some(content),
                },
            );
            Ok(Json(response))
        }
        Err(_) => Err(ApiError::NotFound(format!(
            "Periodic note not found: {} (expected path: {})",
            period, path
        ))),
    }
}
