use axum::{Router, middleware, routing::get};

use crate::auth::auth_middleware;
use crate::state::AppState;

pub mod health;

pub fn routes(state: AppState) -> Router<AppState> {
    let protected = Router::new()
        // Future protected routes go here
        .layer(middleware::from_fn_with_state(state, auth_middleware));

    let public = Router::new()
        .route("/v1/health", get(health::health));

    Router::new()
        .merge(protected)
        .merge(public)
}
