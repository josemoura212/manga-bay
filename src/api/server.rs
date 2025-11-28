use crate::node::app::NodeCommand;
use crate::storage::engine::Storage;
use anyhow::Result;
use tokio::net::TcpListener;
use tokio::sync::mpsc;

pub async fn serve(port: u16, storage: Storage, cmd_tx: mpsc::Sender<NodeCommand>) -> Result<()> {
    let app = super::routes::create_router(storage, cmd_tx);
    let listener = TcpListener::bind(format!("0.0.0.0:{}", port)).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
