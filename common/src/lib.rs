use anyhow::Result;
use libp2p::Multiaddr;
use tokio::sync::oneshot;

// Re-export models
pub use crate::models::*;

pub mod models;
pub mod utils;

#[derive(Debug)]
pub struct NodeInfo {
    pub peer_id: String,
    pub listeners: Vec<String>,
    pub connected_peers: Vec<String>,
}

#[derive(Debug)]
pub enum NodeCommand {
    GetNodeInfo(oneshot::Sender<NodeInfo>),
    ConnectPeer(Multiaddr, oneshot::Sender<Result<()>>),
    FindManga(
        String,
        oneshot::Sender<Option<crate::models::MangaMetadata>>,
    ),
    FindChapters(String, oneshot::Sender<Vec<crate::models::ChapterMetadata>>),
    FindChapterDetails(
        String,
        oneshot::Sender<Option<crate::models::ChapterDetails>>,
    ),
    FindBlock(String, oneshot::Sender<Option<Vec<u8>>>),
    SyncManga(String, oneshot::Sender<Vec<crate::models::MangaVersion>>),
}
