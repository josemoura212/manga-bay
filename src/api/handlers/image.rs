use crate::api::types::{AppError, AppState};
use crate::app::NodeCommand;
use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, HeaderMap},
    response::IntoResponse,
};
use tokio::sync::oneshot;

pub async fn get_image(
    State(state): State<AppState>,
    Path(hash): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    // 1. Try local storage
    match state.storage.get_chunk(&hash).await {
        Ok(Some(data)) => {
            let body = Body::from(data);
            let mut headers = HeaderMap::new();
            headers.insert(header::CONTENT_TYPE, "image/jpeg".parse().unwrap());
            headers.insert(
                header::CACHE_CONTROL,
                "public, max-age=31536000".parse().unwrap(),
            );
            Ok((headers, body))
        }
        Ok(None) => {
            // 2. Try network
            tracing::info!("Image {} not found locally, searching in network...", hash);
            let (tx, rx) = oneshot::channel();
            state
                .cmd_tx
                .send(NodeCommand::FindBlock(hash.clone(), tx))
                .await
                .map_err(|_| {
                    AppError::InternalServerError("Failed to send command to node".to_string())
                })?;

            match tokio::time::timeout(std::time::Duration::from_secs(10), rx).await {
                Ok(Ok(Some(data))) => {
                    // Found in network! Save it.
                    if let Err(e) = state.storage.save_chunk(&hash, &data).await {
                        tracing::error!("Failed to save chunk {}: {:?}", hash, e);
                    }
                    let body = Body::from(data);
                    let mut headers = HeaderMap::new();
                    headers.insert(header::CONTENT_TYPE, "image/jpeg".parse().unwrap());
                    headers.insert(
                        header::CACHE_CONTROL,
                        "public, max-age=31536000".parse().unwrap(),
                    );
                    Ok((headers, body))
                }
                _ => Err(AppError::NotFound(
                    "Image not found locally or in network".to_string(),
                )),
            }
        }
        Err(e) => {
            tracing::error!("Failed to read chunk: {:?}", e);
            Err(AppError::InternalServerError(
                "Failed to read chunk".to_string(),
            ))
        }
    }
}
