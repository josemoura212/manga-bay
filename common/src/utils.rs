use sha2::{Digest, Sha256};

pub mod hash {
    use super::*;

    pub fn calculate_sha256(data: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(data);
        hex::encode(hasher.finalize())
    }
}
