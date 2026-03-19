use axum::{extract::State, response::IntoResponse};
use serde::Serialize;

use crate::response::ApiResponse;
use crate::state::AppState;

#[derive(Serialize)]
pub struct HealthData {
    pub status: &'static str,
    pub vault_name: String,
    pub uptime_seconds: u64,
    pub note_count: usize,
}

pub async fn health(State(state): State<AppState>) -> impl IntoResponse {
    let uptime = state.start_time.elapsed().as_secs();

    let active_vault = state.multi_vault.get_active_vault().await;
    let (vault_name, note_count) = if active_vault.is_empty() {
        ("(none)".to_string(), 0)
    } else {
        let count = match state.multi_vault.get_vault_config(&active_vault).await {
            Ok(config) => count_notes(&config.path),
            Err(_) => 0,
        };
        (active_vault, count)
    };

    ApiResponse::new(&vault_name, "health", HealthData {
        status: "ok",
        vault_name: vault_name.clone(),
        uptime_seconds: uptime,
        note_count,
    })
    .with_count(note_count)
}

/// Count markdown files in a vault directory (non-recursive quick scan).
fn count_notes(path: &std::path::Path) -> usize {
    walkdir(path)
}

fn walkdir(path: &std::path::Path) -> usize {
    let mut count = 0;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                count += walkdir(&p);
            } else if p.extension().is_some_and(|e| e == "md") {
                count += 1;
            }
        }
    }
    count
}
