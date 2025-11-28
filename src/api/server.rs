use crate::storage::engine::Storage;
use anyhow::Result;
use tokio::net::TcpListener;

pub async fn serve(port: u16, storage: Storage) -> Result<()> {
    let app = super::routes::create_router(storage);
    let listener = TcpListener::bind(format!("0.0.0.0:{}", port)).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
