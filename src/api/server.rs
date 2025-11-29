use crate::app::NodeCommand;
use anyhow::Result;
use manga_bay_storage::Storage;
use tokio::net::TcpListener;
use tokio::sync::mpsc;

pub async fn serve(port: u16, storage: Storage, cmd_tx: mpsc::Sender<NodeCommand>) -> Result<()> {
    let app = crate::api::routes::create_router(storage, cmd_tx);
    let listener = TcpListener::bind(format!("0.0.0.0:{}", port)).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
