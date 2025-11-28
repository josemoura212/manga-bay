use anyhow::Result;
use chrono;
use sqlx::{sqlite::SqlitePoolOptions, Pool, Sqlite};
use std::path::PathBuf;
use tokio::fs;

#[derive(Clone)]
pub struct Storage {
    pub pool: Pool<Sqlite>,
    pub data_dir: PathBuf,
}

impl Storage {
    pub async fn new(data_dir: &str) -> Result<Self> {
        let path = PathBuf::from(data_dir);
        if !path.exists() {
            fs::create_dir_all(&path).await?;
        }

        let db_path = path.join("manga.db");
        let db_url = format!("sqlite://{}", db_path.to_string_lossy());

        // Create DB file if not exists
        if !db_path.exists() {
            fs::File::create(&db_path).await?;
        }

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&db_url)
            .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS manga_metadata (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                author TEXT NOT NULL,
                series_code TEXT,
                series_title TEXT,
                alternative_titles TEXT,
                language TEXT NOT NULL,
                created_at INTEGER NOT NULL
            )",
        )
        .execute(&pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS chunk_metadata (
                hash TEXT PRIMARY KEY,
                manga_id TEXT NOT NULL,
                size INTEGER NOT NULL,
                sequence_index INTEGER NOT NULL,
                FOREIGN KEY(manga_id) REFERENCES manga_metadata(id)
            )",
        )
        .execute(&pool)
        .await?;

        Ok(Self {
            pool,
            data_dir: path,
        })
    }

    pub async fn get_mangas(&self) -> Result<Vec<super::models::MangaMetadata>> {
        let mangas =
            sqlx::query_as::<_, super::models::MangaMetadata>("SELECT * FROM manga_metadata")
                .fetch_all(&self.pool)
                .await?;
        Ok(mangas)
    }
}

pub struct IngestParams {
    pub title: String,
    pub author: String,
    pub series_code: Option<String>,
    pub series_title: Option<String>,
    pub alternative_titles: Option<String>,
    pub language: String,
}

impl Storage {
    pub async fn ingest_manga(&self, params: IngestParams, content: Vec<u8>) -> Result<String> {
        let manga_id = uuid::Uuid::new_v4().to_string();
        let created_at = chrono::Utc::now().timestamp();

        // Save metadata
        sqlx::query(
            "INSERT INTO manga_metadata (id, title, author, series_code, series_title, alternative_titles, language, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(&manga_id)
        .bind(&params.title)
        .bind(&params.author)
        .bind(&params.series_code)
        .bind(&params.series_title)
        .bind(&params.alternative_titles)
        .bind(&params.language)
        .bind(created_at)
        .execute(&self.pool)
        .await?;

        // Chunking (simplified: 1MB chunks)
        const CHUNK_SIZE: usize = 1024 * 1024;
        let chunks = content.chunks(CHUNK_SIZE);

        for (i, chunk_data) in chunks.enumerate() {
            let hash = crate::utils::hash::calculate_hash(chunk_data);
            let chunk_path = self.data_dir.join("chunks").join(&hash);

            if let Some(parent) = chunk_path.parent() {
                if !parent.exists() {
                    fs::create_dir_all(parent).await?;
                }
            }

            if !chunk_path.exists() {
                fs::write(&chunk_path, chunk_data).await?;
            }

            sqlx::query(
                "INSERT OR IGNORE INTO chunk_metadata (hash, manga_id, size, sequence_index) VALUES (?, ?, ?, ?)"
            )
            .bind(&hash)
            .bind(&manga_id)
            .bind(chunk_data.len() as i64)
            .bind(i as i32)
            .execute(&self.pool)
            .await?;
        }

        Ok(manga_id)
    }

    pub async fn get_chunk(&self, hash: &str) -> Result<Option<Vec<u8>>> {
        let chunk_path = self.data_dir.join("chunks").join(hash);
        if chunk_path.exists() {
            let data = fs::read(chunk_path).await?;
            Ok(Some(data))
        } else {
            Ok(None)
        }
    }
}
