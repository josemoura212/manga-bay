use anyhow::Result;
use sqlx::{Pool, Sqlite};

#[derive(Clone)]
pub struct RatioManager {
    pool: Pool<Sqlite>,
}

impl RatioManager {
    pub fn new(pool: Pool<Sqlite>) -> Self {
        Self { pool }
    }

    pub async fn init(&self) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS peer_stats (
                peer_id TEXT PRIMARY KEY,
                uploaded_bytes INTEGER DEFAULT 0,
                downloaded_bytes INTEGER DEFAULT 0,
                last_updated INTEGER
            )",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn record_upload(&self, peer_id: &str, bytes: u64) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            "INSERT INTO peer_stats (peer_id, uploaded_bytes, last_updated) 
             VALUES (?, ?, ?)
             ON CONFLICT(peer_id) DO UPDATE SET 
                uploaded_bytes = uploaded_bytes + ?,
                last_updated = ?",
        )
        .bind(peer_id)
        .bind(bytes as i64)
        .bind(now)
        .bind(bytes as i64)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn record_download(&self, peer_id: &str, bytes: u64) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            "INSERT INTO peer_stats (peer_id, downloaded_bytes, last_updated) 
             VALUES (?, ?, ?)
             ON CONFLICT(peer_id) DO UPDATE SET 
                downloaded_bytes = downloaded_bytes + ?,
                last_updated = ?",
        )
        .bind(peer_id)
        .bind(bytes as i64)
        .bind(now)
        .bind(bytes as i64)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_ratio(&self, peer_id: &str) -> Result<f64> {
        let row: (i64, i64) = sqlx::query_as(
            "SELECT uploaded_bytes, downloaded_bytes FROM peer_stats WHERE peer_id = ?",
        )
        .bind(peer_id)
        .fetch_optional(&self.pool)
        .await?
        .unwrap_or((0, 0));

        let uploaded = row.0 as f64;
        let downloaded = row.1 as f64;

        if downloaded == 0.0 {
            return Ok(f64::INFINITY);
        }

        Ok(uploaded / downloaded)
    }

    pub async fn can_consume(&self, peer_id: &str) -> Result<bool> {
        let ratio = self.get_ratio(peer_id).await?;
        // Allow if ratio > 1.0 or if it's a new user (infinite ratio)
        // Also implementing a "free leech" buffer could be done here
        Ok(ratio >= 1.0)
    }
}
