pub mod errors;
pub mod pagination;
pub mod response;
pub mod state;

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
    };

    Router::new().with_state(state)
}
