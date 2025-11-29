use crate::api::types::{AppError, AppState};
use crate::app::NodeCommand;
use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Json,
};
use manga_bay_storage::CreateSeriesParams;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::oneshot;

pub async fn list_mangas(
    State(state): State<AppState>,
) -> Result<Json<Vec<manga_bay_common::models::MangaMetadata>>, AppError> {
    match state.storage.get_mangas().await {
        Ok(mangas) => Ok(Json(mangas)),
        Err(e) => {
            tracing::error!("Failed to list mangas: {:?}", e);
            Err(AppError::InternalServerError(e.to_string()))
        }
    }
}

#[derive(Deserialize)]
pub struct CreateMangaRequest {
    pub title: String,
    pub author: String,
    pub series_code: Option<String>,
    pub series_title: Option<String>,
    pub alternative_titles: Option<Vec<String>>,
}

pub async fn create_manga(
    State(state): State<AppState>,
    Json(payload): Json<CreateMangaRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let alt_titles = payload.alternative_titles.map(|t| t.join(","));

    let params = CreateSeriesParams {
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

pub async fn get_manga(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<manga_bay_common::models::MangaMetadata>, AppError> {
    // 1. Try local storage
    match state.storage.get_manga(&id).await {
        Ok(Some(manga)) => return Ok(Json(manga)),
        Ok(None) => {
            tracing::info!("Manga {} not found locally, querying peers...", id);
        }
        Err(e) => {
            tracing::error!("Failed to get manga locally: {:?}", e);
            return Err(AppError::InternalServerError(e.to_string()));
        }
    }

    // 2. Query Peers
    let (tx, rx) = oneshot::channel();
    state
        .cmd_tx
        .send(NodeCommand::FindManga(id.clone(), tx))
        .await
        .map_err(|_| AppError::InternalServerError("Failed to send command to node".to_string()))?;

    // Wait for response with timeout
    match tokio::time::timeout(std::time::Duration::from_secs(10), rx).await {
        Ok(Ok(Some(manga))) => {
            // Found in network!
            if let Err(e) = sync_manga_metadata(&state, manga.clone()).await {
                tracing::error!("Failed to sync manga metadata: {:?}", e);
            }
            Ok(Json(manga))
        }
        Ok(Ok(None)) => Err(AppError::NotFound("Manga not found in network".to_string())),
        Ok(Err(_)) => Err(AppError::InternalServerError(
            "Failed to receive response from node".to_string(),
        )),
        Err(_) => Err(AppError::NotFound("Manga not found (timeout)".to_string())),
    }
}

pub async fn sync_manga(
    State(state): State<AppState>,
    Path(manga_id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    // 1. Calculate local version
    let local_version = state
        .storage
        .calculate_manga_version(&manga_id)
        .await
        .map_err(|e| AppError::InternalServerError(e.to_string()))?;

    // 2. Query peers for their version
    let (tx, rx) = oneshot::channel();
    state
        .cmd_tx
        .send(NodeCommand::SyncManga(manga_id.clone(), tx))
        .await
        .map_err(|_| AppError::InternalServerError("Failed to send command".to_string()))?;

    let remote_versions = match tokio::time::timeout(std::time::Duration::from_secs(5), rx).await {
        Ok(Ok(v)) => v,
        _ => {
            return Err(AppError::InternalServerError(
                "Timeout or error fetching remote versions".to_string(),
            ))
        }
    };

    if remote_versions.is_empty() {
        return Ok(Json(
            json!({ "status": "no_peers", "message": "No peers found with this manga" }),
        ));
    }

    let remote_versions_count = remote_versions.len();

    // 3. Compare and Sync
    let mut synced = false;
    for remote in remote_versions {
        let should_sync = match &local_version {
            None => true, // We don't have it, so sync
            Some(local) => {
                remote.chapter_count > local.chapter_count
                    || (remote.chapter_count == local.chapter_count && remote.hash != local.hash)
            }
        };

        if should_sync {
            tracing::info!(
                "Syncing manga {} from peer (Remote: {} ch, Local: {:?} ch)",
                manga_id,
                remote.chapter_count,
                local_version.as_ref().map(|v| v.chapter_count)
            );

            let (tx_manga, rx_manga) = oneshot::channel();
            state
                .cmd_tx
                .send(NodeCommand::FindManga(manga_id.clone(), tx_manga))
                .await
                .map_err(|_| {
                    AppError::InternalServerError("Failed to fetch manga metadata".to_string())
                })?;

            if let Ok(Ok(Some(manga))) =
                tokio::time::timeout(std::time::Duration::from_secs(10), rx_manga).await
            {
                if let Err(e) = sync_manga_metadata(&state, manga).await {
                    tracing::error!("Failed to sync metadata: {:?}", e);
                } else {
                    synced = true;
                }
            }
        }
    }

    Ok(Json(json!({
        "status": "success",
        "synced": synced,
        "local_version": local_version,
        "remote_versions_count": remote_versions_count
    })))
}

async fn sync_manga_metadata(
    state: &AppState,
    manga: manga_bay_common::models::MangaMetadata,
) -> anyhow::Result<()> {
    tracing::info!("Syncing metadata for manga: {}", manga.title);
    state.storage.save_manga(manga.clone()).await?;

    // Get Chapters
    let (tx, rx) = oneshot::channel();
    state
        .cmd_tx
        .send(NodeCommand::FindChapters(manga.id.clone(), tx))
        .await?;

    let chapters = match tokio::time::timeout(std::time::Duration::from_secs(5), rx).await {
        Ok(Ok(c)) => c,
        _ => {
            tracing::warn!("Timeout or error fetching chapters for {}", manga.id);
            return Ok(());
        }
    };

    for chapter in chapters {
        state.storage.save_chapter_metadata(chapter.clone()).await?;

        // Get Details
        let (tx, rx) = oneshot::channel();
        state
            .cmd_tx
            .send(NodeCommand::FindChapterDetails(chapter.id.clone(), tx))
            .await?;

        if let Ok(Ok(Some(details))) =
            tokio::time::timeout(std::time::Duration::from_secs(5), rx).await
        {
            for (i, hash) in details.pages.iter().enumerate() {
                let chunk = manga_bay_common::models::ChunkMetadata {
                    chapter_id: chapter.id.clone(),
                    hash: hash.clone(),
                    sequence_index: i as i32,
                    size: 0, // Placeholder until downloaded
                };
                state.storage.save_chunk_metadata(chunk).await?;
            }
        }
    }
    tracing::info!("Finished syncing metadata for {}", manga.title);
    Ok(())
}
