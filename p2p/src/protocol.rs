use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppRequest {
    GetBlock { block_hash: String },
    GetManga { manga_id: String },
    GetChapters { manga_id: String },
    GetChapterDetails { chapter_id: String },
    GetVersion { manga_id: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AppResponse {
    Block(String, Option<Vec<u8>>),
    Manga(Option<manga_bay_common::models::MangaMetadata>),
    Chapters(Vec<manga_bay_common::models::ChapterMetadata>),
    ChapterDetails(Option<manga_bay_common::models::ChapterDetails>),
    Version(Option<manga_bay_common::models::MangaVersion>),
}
