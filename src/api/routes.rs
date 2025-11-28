use crate::storage::engine::Storage;
use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose, Engine as _};
use serde::Deserialize;

pub fn create_router(storage: Storage) -> Router {
    Router::new()
        .route("/health", get(health_check))
        .route("/mangas", get(list_mangas).post(create_manga))
        .with_state(storage)
}

async fn health_check() -> &'static str {
    "OK"
}

async fn list_mangas(
    State(storage): State<Storage>,
) -> Json<Vec<crate::storage::models::MangaMetadata>> {
    match storage.get_mangas().await {
        Ok(mangas) => Json(mangas),
        Err(_) => Json(vec![]),
    }
}

use std::io::Write;

#[derive(Deserialize)]
struct CreateMangaRequest {
    title: String,
    author: String,
    series_code: Option<String>,
    series_title: Option<String>,
    alternative_titles: Option<Vec<String>>,
    language: String,
    content_base64: Option<String>,
    pages: Option<Vec<String>>,
}

async fn create_manga(
    State(storage): State<Storage>,
    Json(payload): Json<CreateMangaRequest>,
) -> Result<Json<String>, StatusCode> {
    let content = if let Some(pages) = payload.pages {
        // Create ZIP from pages
        let mut buf = Vec::new();
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        let options =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);

        for (i, page_base64) in pages.iter().enumerate() {
            let page_data = general_purpose::STANDARD
                .decode(page_base64)
                .map_err(|_| StatusCode::BAD_REQUEST)?;

            let filename = format!("page_{:03}.jpg", i + 1);
            zip.start_file(filename, options)
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            zip.write_all(&page_data)
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
        zip.finish()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        drop(zip);
        buf
    } else if let Some(content_base64) = payload.content_base64 {
        general_purpose::STANDARD
            .decode(&content_base64)
            .map_err(|_| StatusCode::BAD_REQUEST)?
    } else {
        return Err(StatusCode::BAD_REQUEST);
    };

    let alt_titles = payload.alternative_titles.map(|t| t.join(","));

    let params = crate::storage::engine::IngestParams {
        title: payload.title,
        author: payload.author,
        series_code: payload.series_code,
        series_title: payload.series_title,
        alternative_titles: alt_titles,
        language: payload.language,
    };

    match storage.ingest_manga(params, content).await {
        Ok(id) => Ok(Json(id)),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}
