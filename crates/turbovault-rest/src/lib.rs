pub mod auth;
pub mod content;
pub mod errors;
pub mod pagination;
pub mod response;
pub mod state;
pub mod v1;
pub mod vault_resolver;

use axum::Router;
use state::AppState;
use std::sync::Arc;
use turbovault_core::prelude::MultiVaultManager;

pub use state::RestConfig;

/// Build the REST API router. Merge with MCP router in main.rs.
pub fn router(multi_vault: Arc<MultiVaultManager>, config: RestConfig) -> Router {
    let state = AppState {
        multi_vault,
        config,
        start_time: std::time::Instant::now(),
        vault_managers: std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
    };

    v1::routes(state.clone()).with_state(state)
}
