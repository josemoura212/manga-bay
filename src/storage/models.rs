use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct MangaMetadata {
    pub id: String,
    pub title: String,
    pub author: String,
    pub series_code: Option<String>,
    pub series_title: Option<String>,
    pub alternative_titles: Option<String>,
    pub language: String,
    pub created_at: i64,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct ChunkMetadata {
    pub hash: String,
    pub manga_id: String,
    pub size: i64,
    pub sequence_index: i32,
}
