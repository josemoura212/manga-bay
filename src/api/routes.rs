use crate::node::app::NodeCommand;
use crate::storage::engine::Storage;
use axum::{
    body::Body,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose, Engine as _};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::io::Write;
use tokio::sync::{mpsc, oneshot};

#[derive(Clone)]
pub struct AppState {
    pub storage: Storage,
    pub cmd_tx: mpsc::Sender<NodeCommand>,
}

// Standardized Error Response
pub enum AppError {
    InternalServerError(String),
    BadRequest(String),
    NotFound(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            AppError::InternalServerError(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
        };

        let body = Json(json!({
            "status": "error",
            "message": message
        }));

        (status, body).into_response()
    }
}

pub fn create_router(storage: Storage, cmd_tx: mpsc::Sender<NodeCommand>) -> Router {
    let state = AppState { storage, cmd_tx };
    Router::new()
        .route("/health", get(health_check))
        .route("/mangas", get(list_mangas).post(create_manga))
        .route("/mangas/:id", get(get_manga))
        .route("/mangas/:id/chapters", get(list_chapters))
        .route("/chapters", post(create_chapter))
        .route("/chapters/:id", get(get_chapter))
        .route("/images/:hash", get(get_image))
        .route("/node", get(get_node_info))
        .route("/peers", post(connect_peer))
        .with_state(state)
}

async fn health_check() -> &'static str {
    "OK"
}

#[derive(Serialize)]
struct NodeInfoResponse {
    peer_id: String,
    listeners: Vec<String>,
}

async fn get_node_info(State(state): State<AppState>) -> Result<Json<NodeInfoResponse>, AppError> {
    let (tx, rx) = oneshot::channel();
    state
        .cmd_tx
        .send(NodeCommand::GetNodeInfo(tx))
        .await
        .map_err(|_| AppError::InternalServerError("Failed to send command to node".to_string()))?;

    let info = rx.await.map_err(|_| {
        AppError::InternalServerError("Failed to receive response from node".to_string())
    })?;
    Ok(Json(NodeInfoResponse {
        peer_id: info.peer_id,
        listeners: info.listeners,
    }))
}

#[derive(Deserialize)]
struct ConnectPeerRequest {
    multiaddr: String,
}

async fn connect_peer(
    State(state): State<AppState>,
    Json(payload): Json<ConnectPeerRequest>,
) -> Result<StatusCode, AppError> {
    let addr: libp2p::Multiaddr = payload
        .multiaddr
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid multiaddr".to_string()))?;

    let (tx, rx) = oneshot::channel();
    state
        .cmd_tx
        .send(NodeCommand::ConnectPeer(addr, tx))
        .await
        .map_err(|_| AppError::InternalServerError("Failed to send command to node".to_string()))?;

    match rx.await.map_err(|_| {
        AppError::InternalServerError("Failed to receive response from node".to_string())
    })? {
        Ok(_) => Ok(StatusCode::OK),
        Err(_) => Err(AppError::InternalServerError(
            "Failed to connect to peer".to_string(),
        )),
    }
}

async fn list_mangas(
    State(state): State<AppState>,
) -> Result<Json<Vec<crate::storage::models::MangaMetadata>>, AppError> {
    match state.storage.get_mangas().await {
        Ok(mangas) => Ok(Json(mangas)),
        Err(e) => {
            tracing::error!("Failed to list mangas: {:?}", e);
            Err(AppError::InternalServerError(e.to_string()))
        }
    }
}

#[derive(Deserialize)]
struct CreateMangaRequest {
    title: String,
    author: String,
    series_code: Option<String>,
    series_title: Option<String>,
    alternative_titles: Option<Vec<String>>,
}

async fn create_manga(
    State(state): State<AppState>,
    Json(payload): Json<CreateMangaRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let alt_titles = payload.alternative_titles.map(|t| t.join(","));

    let params = crate::storage::engine::CreateSeriesParams {
        title: payload.title,
        author: payload.author,
        series_code: payload.series_code,
        series_title: payload.series_title,
        alternative_titles: alt_titles,
    };

    match state.storage.create_series(params).await {
        Ok(id) => Ok(Json(json!({ "id": id }))),
        Err(e) => {
            tracing::error!("Failed to create series: {:?}", e);
            Err(AppError::InternalServerError(e.to_string()))
        }
    }
}

#[derive(Deserialize)]
struct CreateChapterPage {
    index: i32,
    content: String,
}

#[derive(Deserialize)]
struct CreateChapterRequest {
    manga_id: String,
    title: String,
    chapter_number: f64,
    language: String,
    pages: Vec<CreateChapterPage>,
}

async fn create_chapter(
    State(state): State<AppState>,
    Json(payload): Json<CreateChapterRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let mut pages_data = Vec::new();

    for (i, page) in payload.pages.iter().enumerate() {
        // Handle Data URI scheme
        let clean_base64 = if let Some(index) = page.content.find(',') {
            &page.content[index + 1..]
        } else {
            &page.content
        };

        let clean_base64: String = clean_base64
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();

        let page_data = general_purpose::STANDARD
            .decode(&clean_base64)
            .map_err(|e| AppError::BadRequest(format!("Invalid Base64 page {}: {}", i, e)))?;

        tracing::info!("Page {} decoded size: {} bytes", i, page_data.len());
        pages_data.push((page.index, page_data));
    }

    let params = crate::storage::engine::IngestChapterParams {
        manga_id: payload.manga_id,
        title: payload.title,
        chapter_number: payload.chapter_number,
        language: payload.language,
    };

    match state.storage.ingest_chapter(params, pages_data).await {
        Ok(id) => Ok(Json(json!({ "id": id }))),
        Err(e) => {
            tracing::error!("Failed to ingest chapter: {:?}", e);
            Err(AppError::InternalServerError(e.to_string()))
        }
    }
}

async fn get_manga(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<crate::storage::models::MangaMetadata>, AppError> {
    match state.storage.get_manga(&id).await {
        Ok(Some(manga)) => Ok(Json(manga)),
        Ok(None) => Err(AppError::NotFound("Manga not found".to_string())),
        Err(e) => {
            tracing::error!("Failed to get manga: {:?}", e);
            Err(AppError::InternalServerError(e.to_string()))
        }
    }
}

async fn list_chapters(
    State(state): State<AppState>,
    Path(manga_id): Path<String>,
) -> Result<Json<Vec<crate::storage::models::ChapterMetadata>>, AppError> {
    match state.storage.list_chapters(&manga_id).await {
        Ok(chapters) => Ok(Json(chapters)),
        Err(e) => {
            tracing::error!("Failed to list chapters: {:?}", e);
            Err(AppError::InternalServerError(e.to_string()))
        }
    }
}

async fn get_chapter(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<crate::storage::models::ChapterDetails>, AppError> {
    match state.storage.get_chapter_details(&id).await {
        Ok(Some(chapter)) => Ok(Json(chapter)),
        Ok(None) => Err(AppError::NotFound("Chapter not found".to_string())),
        Err(e) => {
            tracing::error!("Failed to get chapter: {:?}", e);
            Err(AppError::InternalServerError(e.to_string()))
        }
    }
}

fn detect_mime_type(data: &[u8]) -> &'static str {
    if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if data.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        "image/png"
    } else if data.starts_with(&[0x47, 0x49, 0x46, 0x38]) {
        "image/gif"
    } else if data.starts_with(&[0x52, 0x49, 0x46, 0x46])
        && data.len() > 12
        && &data[8..12] == b"WEBP"
    {
        "image/webp"
    } else {
        "application/octet-stream"
    }
}

async fn get_image(
    State(state): State<AppState>,
    Path(hash): Path<String>,
) -> Result<Response, AppError> {
    match state.storage.get_chunk(&hash).await {
        Ok(Some(data)) => {
            let mime_type = detect_mime_type(&data);
            tracing::info!(
                "Serving image {}: {} bytes, type: {}",
                hash,
                data.len(),
                mime_type
            );

            let body = Body::from(data);
            Ok(Response::builder()
                .header("Content-Type", mime_type)
                .body(body)
                .unwrap())
        }
        Ok(None) => Err(AppError::NotFound("Image not found".to_string())),
        Err(e) => {
            tracing::error!("Failed to get image: {:?}", e);
            Err(AppError::InternalServerError(e.to_string()))
        }
    }
}
