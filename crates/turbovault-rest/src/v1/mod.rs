use axum::{Router, middleware, routing::{get, post}};

use crate::auth::auth_middleware;
use crate::state::AppState;

pub mod batch;
pub mod files;
pub mod health;
pub mod links;
pub mod notes;
pub mod notes_info;
pub mod periodic;
pub mod recent;
pub mod search;
pub mod trash;

pub fn routes(state: AppState) -> Router<AppState> {
    let protected = Router::new()
        .route("/v1/notes/{*path}", get(notes::read_note).put(notes::create_note).post(notes::append_note).patch(notes::patch_note).delete(notes::delete_note))
        .route("/v1/notes-info/{*path}", get(notes_info::get_info))
        .route("/v1/search", get(search::search))
        .route("/v1/trash", get(trash::list_trash))
        .route("/v1/restore/{*path}", post(trash::restore))
        .route("/v1/request-purge/{*path}", post(trash::request_purge))
        .route("/v1/files", get(files::list_root))
        .route("/v1/files/{*path}", get(files::list_dir))
        .route("/v1/periodic/{period}", get(periodic::get_periodic))
        .route("/v1/recent", get(recent::get_recent))
        .route("/v1/backlinks/{*path}", get(links::backlinks))
        .route("/v1/forward-links/{*path}", get(links::forward_links))
        .route("/v1/batch/read", post(batch::batch_read))
        .layer(middleware::from_fn_with_state(state, auth_middleware));

    let public = Router::new()
        .route("/v1/health", get(health::health));

    Router::new()
        .merge(protected)
        .merge(public)
}
