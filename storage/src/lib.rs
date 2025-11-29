use anyhow::Result;

use manga_bay_common::models;
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

        // Manga Series Table
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS manga_metadata (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                author TEXT NOT NULL,
                series_code TEXT,
                series_title TEXT,
                alternative_titles TEXT,
                created_at INTEGER NOT NULL
            )",
        )
        .execute(&pool)
        .await?;

        // Chapters Table
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS chapters (
                id TEXT PRIMARY KEY,
                manga_id TEXT NOT NULL,
                title TEXT NOT NULL,
                chapter_number REAL NOT NULL,
                language TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                FOREIGN KEY(manga_id) REFERENCES manga_metadata(id),
                UNIQUE(manga_id, chapter_number, language)
            )",
        )
        .execute(&pool)
        .await?;

        // Chunks Table (linked to chapters)
        // Note: We use (chapter_id, sequence_index) as PK to allow the same hash (image)
        // to be used in multiple chapters or multiple times in the same chapter.
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS chunk_metadata (
                chapter_id TEXT NOT NULL,
                hash TEXT NOT NULL,
                sequence_index INTEGER NOT NULL,
                size INTEGER NOT NULL,
                PRIMARY KEY (chapter_id, sequence_index),
                FOREIGN KEY(chapter_id) REFERENCES chapters(id)
            )",
        )
        .execute(&pool)
        .await?;

        Ok(Self {
            pool,
            data_dir: path,
        })
    }

    pub async fn get_mangas(&self) -> Result<Vec<models::MangaMetadata>> {
        let mangas = sqlx::query_as::<_, models::MangaMetadata>("SELECT * FROM manga_metadata")
            .fetch_all(&self.pool)
            .await?;
        Ok(mangas)
    }
}

pub struct CreateSeriesParams {
    pub title: String,
    pub author: String,
    pub series_code: Option<String>,
    pub series_title: Option<String>,
    pub alternative_titles: Option<String>,
}

pub struct IngestChapterParams {
    pub manga_id: String,
    pub title: String,
    pub chapter_number: f64,
    pub language: String,
}

impl Storage {
    pub async fn create_series(&self, params: CreateSeriesParams) -> Result<String> {
        // Check if manga already exists
        let existing = sqlx::query_as::<_, models::MangaMetadata>(
            "SELECT * FROM manga_metadata WHERE title = ? AND author = ?",
        )
        .bind(&params.title)
        .bind(&params.author)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(manga) = existing {
            tracing::info!("Manga already exists: {} ({})", manga.title, manga.id);
            return Ok(manga.id);
        }

        let manga_id = uuid::Uuid::new_v4().to_string();
        let created_at = chrono::Utc::now().timestamp();

        sqlx::query(
            "INSERT INTO manga_metadata (id, title, author, series_code, series_title, alternative_titles, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(&manga_id)
        .bind(&params.title)
        .bind(&params.author)
        .bind(&params.series_code)
        .bind(&params.series_title)
        .bind(&params.alternative_titles)
        .bind(created_at)
        .execute(&self.pool)
        .await?;

        Ok(manga_id)
    }

    pub async fn save_manga(&self, manga: models::MangaMetadata) -> Result<()> {
        // Check if it already exists by ID
        let exists = sqlx::query("SELECT 1 FROM manga_metadata WHERE id = ?")
            .bind(&manga.id)
            .fetch_optional(&self.pool)
            .await?;

        if exists.is_some() {
            return Ok(());
        }

        sqlx::query(
            "INSERT INTO manga_metadata (id, title, author, series_code, series_title, alternative_titles, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(&manga.id)
        .bind(&manga.title)
        .bind(&manga.author)
        .bind(&manga.series_code)
        .bind(&manga.series_title)
        .bind(&manga.alternative_titles)
        .bind(manga.created_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn save_chapter_metadata(&self, chapter: models::ChapterMetadata) -> Result<()> {
        let exists = sqlx::query("SELECT 1 FROM chapters WHERE id = ?")
            .bind(&chapter.id)
            .fetch_optional(&self.pool)
            .await?;

        if exists.is_some() {
            return Ok(());
        }

        sqlx::query(
            "INSERT INTO chapters (id, manga_id, title, chapter_number, language, created_at) VALUES (?, ?, ?, ?, ?, ?)"
        )
        .bind(&chapter.id)
        .bind(&chapter.manga_id)
        .bind(&chapter.title)
        .bind(chapter.chapter_number)
        .bind(&chapter.language)
        .bind(chapter.created_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn save_chunk_metadata(&self, chunk: models::ChunkMetadata) -> Result<()> {
        sqlx::query(
            "INSERT OR IGNORE INTO chunk_metadata (chapter_id, hash, sequence_index, size) VALUES (?, ?, ?, ?)"
        )
        .bind(&chunk.chapter_id)
        .bind(&chunk.hash)
        .bind(chunk.sequence_index)
        .bind(chunk.size)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn save_chunk(&self, hash: &str, data: &[u8]) -> Result<()> {
        let chunk_path = self.data_dir.join("chunks").join(hash);
        if let Some(parent) = chunk_path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).await?;
            }
        }
        if !chunk_path.exists() {
            fs::write(&chunk_path, data).await?;
        }
        Ok(())
    }

    pub async fn calculate_manga_version(
        &self,
        manga_id: &str,
    ) -> Result<Option<models::MangaVersion>> {
        let manga = match self.get_manga(manga_id).await? {
            Some(m) => m,
            None => return Ok(None),
        };

        let chapters = self.list_chapters(manga_id).await?;

        // Simple hash: Title + Number of Chapters + Sorted Chapter IDs
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        use std::hash::{Hash, Hasher};

        manga.title.hash(&mut hasher);
        chapters.len().hash(&mut hasher);

        let mut sorted_chapters: Vec<_> = chapters.iter().collect();
        sorted_chapters.sort_by_key(|c| &c.id);

        for chapter in sorted_chapters {
            chapter.id.hash(&mut hasher);
            chapter.created_at.hash(&mut hasher);
        }

        let hash = format!("{:x}", hasher.finish());

        Ok(Some(models::MangaVersion {
            manga_id: manga_id.to_string(),
            hash,
            chapter_count: chapters.len(),
            created_at: manga.created_at,
        }))
    }

    pub async fn ingest_chapter(
        &self,
        params: IngestChapterParams,
        pages: Vec<(i32, Vec<u8>)>,
    ) -> Result<String> {
        // Check if chapter already exists
        let existing_chapter = sqlx::query_as::<_, models::ChapterMetadata>(
            "SELECT * FROM chapters WHERE manga_id = ? AND chapter_number = ? AND language = ?",
        )
        .bind(&params.manga_id)
        .bind(params.chapter_number)
        .bind(&params.language)
        .fetch_optional(&self.pool)
        .await?;

        let chapter_id = if let Some(chapter) = existing_chapter {
            // Update existing chapter
            tracing::info!(
                "Updating existing chapter {} (number {})",
                chapter.id,
                params.chapter_number
            );

            // Delete old chunks metadata (we will overwrite them)
            sqlx::query("DELETE FROM chunk_metadata WHERE chapter_id = ?")
                .bind(&chapter.id)
                .execute(&self.pool)
                .await?;

            // Update chapter metadata
            sqlx::query("UPDATE chapters SET title = ?, language = ? WHERE id = ?")
                .bind(&params.title)
                .bind(&params.language)
                .bind(&chapter.id)
                .execute(&self.pool)
                .await?;

            chapter.id
        } else {
            // Create new chapter
            let new_id = uuid::Uuid::new_v4().to_string();
            let created_at = chrono::Utc::now().timestamp();

            sqlx::query(
                "INSERT INTO chapters (id, manga_id, title, chapter_number, language, created_at) VALUES (?, ?, ?, ?, ?, ?)"
            )
            .bind(&new_id)
            .bind(&params.manga_id)
            .bind(&params.title)
            .bind(params.chapter_number)
            .bind(&params.language)
            .bind(created_at)
            .execute(&self.pool)
            .await?;

            new_id
        };

        // Store each page as a chunk
        for (index, page_data) in pages {
            let hash = manga_bay_common::utils::hash::calculate_sha256(&page_data);
            let chunk_path = self.data_dir.join("chunks").join(&hash);

            if let Some(parent) = chunk_path.parent() {
                if !parent.exists() {
                    fs::create_dir_all(parent).await?;
                }
            }

            if !chunk_path.exists() {
                fs::write(&chunk_path, &page_data).await?;
            }

            sqlx::query(
                "INSERT OR IGNORE INTO chunk_metadata (chapter_id, hash, sequence_index, size) VALUES (?, ?, ?, ?)"
            )
            .bind(&chapter_id)
            .bind(&hash)
            .bind(index)
            .bind(page_data.len() as i64)
            .execute(&self.pool)
            .await?;
        }

        Ok(chapter_id)
    }

    pub async fn get_chapter_details(&self, id: &str) -> Result<Option<models::ChapterDetails>> {
        let chapter =
            sqlx::query_as::<_, models::ChapterMetadata>("SELECT * FROM chapters WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;

        if let Some(chapter) = chapter {
            // Fetch pages (chunks) ordered by sequence_index
            let chunks = sqlx::query_as::<_, models::ChunkMetadata>(
                "SELECT * FROM chunk_metadata WHERE chapter_id = ? ORDER BY sequence_index ASC",
            )
            .bind(id)
            .fetch_all(&self.pool)
            .await?;

            let pages = chunks.into_iter().map(|c| c.hash).collect();

            Ok(Some(models::ChapterDetails {
                id: chapter.id,
                manga_id: chapter.manga_id,
                title: chapter.title,
                chapter_number: chapter.chapter_number,
                language: chapter.language,
                created_at: chapter.created_at,
                pages,
            }))
        } else {
            Ok(None)
        }
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

    pub async fn get_manga(&self, id: &str) -> Result<Option<models::MangaMetadata>> {
        let manga =
            sqlx::query_as::<_, models::MangaMetadata>("SELECT * FROM manga_metadata WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(manga)
    }

    pub async fn get_chapter(&self, id: &str) -> Result<Option<models::ChapterMetadata>> {
        let chapter =
            sqlx::query_as::<_, models::ChapterMetadata>("SELECT * FROM chapters WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(chapter)
    }

    pub async fn list_chapters(&self, manga_id: &str) -> Result<Vec<models::ChapterMetadata>> {
        let chapters = sqlx::query_as::<_, models::ChapterMetadata>(
            "SELECT * FROM chapters WHERE manga_id = ? ORDER BY chapter_number ASC",
        )
        .bind(manga_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(chapters)
    }
}
