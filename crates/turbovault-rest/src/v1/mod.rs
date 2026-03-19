use axum::{Router, middleware, routing::{get, post}};

use crate::auth::auth_middleware;
use crate::state::AppState;

pub mod health;
pub mod notes;
pub mod notes_info;
pub mod trash;

pub fn routes(state: AppState) -> Router<AppState> {
    let protected = Router::new()
        .route("/v1/notes/{*path}", get(notes::read_note).put(notes::create_note).post(notes::append_note).patch(notes::patch_note).delete(notes::delete_note))
        .route("/v1/notes-info/{*path}", get(notes_info::get_info))
        .route("/v1/trash", get(trash::list_trash))
        .route("/v1/restore/{*path}", post(trash::restore))
        .route("/v1/request-purge/{*path}", post(trash::request_purge))
        .layer(middleware::from_fn_with_state(state, auth_middleware));

    let public = Router::new()
        .route("/v1/health", get(health::health));

    Router::new()
        .merge(protected)
        .merge(public)
}
