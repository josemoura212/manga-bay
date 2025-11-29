use crate::api::handlers::{chapter, image, manga, node};
use crate::api::types::AppState;
use crate::app::NodeCommand;
use axum::{
    routing::{get, post},
    Router,
};
use manga_bay_storage::Storage;
use tokio::sync::mpsc;

pub fn create_router(storage: Storage, cmd_tx: mpsc::Sender<NodeCommand>) -> Router {
    let state = AppState { storage, cmd_tx };
    Router::new()
        .route("/health", get(node::health_check))
        // Manga routes (singular)
        .route("/manga", get(manga::list_mangas).post(manga::create_manga))
        .route("/manga/{id}", get(manga::get_manga))
        .route("/manga/{id}/chapters", get(chapter::list_chapters))
        .route("/manga/{id}/sync", post(manga::sync_manga))
        // Chapter routes
        .route("/chapter", post(chapter::create_chapter))
        .route("/chapter/{id}", get(chapter::get_chapter))
        // Image routes
        .route("/image/{hash}", get(image::get_image))
        // Node routes
        .route("/node", get(node::get_node_info))
        .route("/peer", post(node::connect_peer))
        .with_state(state)
}
