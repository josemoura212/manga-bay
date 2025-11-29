use anyhow::Result;
use libp2p::Multiaddr;

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use tokio::fs;

#[derive(Debug, Serialize, Deserialize)]
pub struct KnownPeers {
    pub peers: HashSet<String>, // Multiaddrs as strings
}

pub struct PeerPersistence {
    file_path: PathBuf,
}

impl PeerPersistence {
    pub fn new(data_dir: &std::path::Path) -> Self {
        Self {
            file_path: data_dir.join("peers.json"),
        }
    }

    pub async fn load(&self) -> Result<Vec<Multiaddr>> {
        if !self.file_path.exists() {
            return Ok(Vec::new());
        }

        let content = fs::read_to_string(&self.file_path).await?;
        let known: KnownPeers = serde_json::from_str(&content)?;

        let mut multiaddrs = Vec::new();
        for s in known.peers {
            if let Ok(ma) = s.parse() {
                multiaddrs.push(ma);
            }
        }

        Ok(multiaddrs)
    }

    pub async fn save(&self, peers: Vec<Multiaddr>) -> Result<()> {
        let set: HashSet<String> = peers.into_iter().map(|ma| ma.to_string()).collect();
        let known = KnownPeers { peers: set };
        let content = serde_json::to_string_pretty(&known)?;
        fs::write(&self.file_path, content).await?;
        Ok(())
    }
}
