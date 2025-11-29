use libp2p::Multiaddr;
use std::collections::HashMap;
use tokio::sync::oneshot; // Added this to resolve the types below

// We will move the command handling logic here, but it needs access to the swarm and pending maps.
// Since `Swarm` and the maps are local variables in `run`, passing them all might be cumbersome.
// A better approach for `app.rs` refactoring might be to encapsulate the state in a struct.

pub struct NodeState {
    pub pending_manga_requests:
        HashMap<String, Vec<oneshot::Sender<Option<manga_bay_common::models::MangaMetadata>>>>,
    pub pending_chapters_requests:
        HashMap<String, Vec<oneshot::Sender<Vec<manga_bay_common::models::ChapterMetadata>>>>,
    pub pending_chapter_details_requests:
        HashMap<String, Vec<oneshot::Sender<Option<manga_bay_common::models::ChapterDetails>>>>,
    pub pending_block_requests: HashMap<String, Vec<oneshot::Sender<Option<Vec<u8>>>>>,
    pub pending_version_requests:
        HashMap<String, Vec<oneshot::Sender<Vec<manga_bay_common::models::MangaVersion>>>>,
    pub known_peers: Vec<Multiaddr>,
}

impl Default for NodeState {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeState {
    pub fn new() -> Self {
        Self {
            pending_manga_requests: HashMap::new(),
            pending_chapters_requests: HashMap::new(),
            pending_chapter_details_requests: HashMap::new(),
            pending_block_requests: HashMap::new(),
            pending_version_requests: HashMap::new(),
            known_peers: Vec::new(),
        }
    }
}
