use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppRequest {
    GetBlock { block_hash: String },
    GetManga { manga_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppResponse {
    Block(Option<Vec<u8>>),
    Manga(Option<crate::storage::models::MangaMetadata>),
}
