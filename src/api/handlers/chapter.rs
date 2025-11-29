use crate::api::types::{AppError, AppState};
use axum::{
    extract::{Path, State},
    Json,
};
use base64::{engine::general_purpose, Engine as _};
use manga_bay_common::models;
use manga_bay_storage::IngestChapterParams;
use serde::Deserialize;
use serde_json::json;

pub async fn list_chapters(
    State(state): State<AppState>,
    Path(manga_id): Path<String>,
) -> Result<Json<Vec<models::ChapterMetadata>>, AppError> {
    match state.storage.list_chapters(&manga_id).await {
        Ok(chapters) => Ok(Json(chapters)),
        Err(e) => {
            tracing::error!("Failed to list chapters: {:?}", e);
            Err(AppError::InternalServerError(e.to_string()))
        }
    }
}

#[derive(Deserialize)]
pub struct CreateChapterPage {
    pub index: i32,
    pub content: String,
}

#[derive(Deserialize)]
pub struct CreateChapterRequest {
    pub manga_id: String,
    pub title: String,
    pub chapter_number: f64,
    pub language: String,
    pub pages: Vec<CreateChapterPage>,
}

pub async fn create_chapter(
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

    let params = IngestChapterParams {
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

pub async fn get_chapter(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<manga_bay_common::models::ChapterDetails>, AppError> {
    match state.storage.get_chapter_details(&id).await {
        Ok(Some(chapter)) => Ok(Json(chapter)),
        Ok(None) => Err(AppError::NotFound("Chapter not found".to_string())),
        Err(e) => {
            tracing::error!("Failed to get chapter: {:?}", e);
            Err(AppError::InternalServerError(e.to_string()))
        }
    }
}
