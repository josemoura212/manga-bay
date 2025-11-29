use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Serialize, Deserialize, FromRow, PartialEq, Eq, Clone)]
pub struct MangaMetadata {
    pub id: String,
    pub title: String,
    pub author: String,
    pub series_code: Option<String>,
    pub series_title: Option<String>,
    pub alternative_titles: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct ChunkMetadata {
    pub hash: String,
    pub chapter_id: String,
    pub size: i64,
    pub sequence_index: i32,
}

#[derive(Debug, Serialize, Deserialize, FromRow, Clone, PartialEq)]
pub struct ChapterMetadata {
    pub id: String,
    pub manga_id: String,
    pub title: String,
    pub chapter_number: f64,
    pub language: String,
    pub created_at: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct ChapterDetails {
    pub id: String,
    pub manga_id: String,
    pub title: String,
    pub chapter_number: f64,
    pub language: String,
    pub created_at: i64,
    pub pages: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct MangaVersion {
    pub manga_id: String,
    pub hash: String,
    pub chapter_count: usize,
    pub created_at: i64,
}
