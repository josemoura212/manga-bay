use crate::api::types::{AppError, AppState};
use crate::app::NodeCommand;
use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

pub async fn health_check() -> &'static str {
    "OK"
}

#[derive(Serialize)]
pub struct NodeInfoResponse {
    pub peer_id: String,
    pub listeners: Vec<String>,
    pub connected_peers: Vec<String>,
}

pub async fn get_node_info(
    State(state): State<AppState>,
) -> Result<Json<NodeInfoResponse>, AppError> {
    let (tx, rx) = oneshot::channel();
    state
        .cmd_tx
        .send(NodeCommand::GetNodeInfo(tx))
        .await
        .map_err(|_| AppError::InternalServerError("Failed to send command to node".to_string()))?;

    let info = rx.await.map_err(|_| {
        AppError::InternalServerError("Failed to receive response from node".to_string())
    })?;
    Ok(Json(NodeInfoResponse {
        peer_id: info.peer_id,
        listeners: info.listeners,
        connected_peers: info.connected_peers,
    }))
}

#[derive(Deserialize)]
pub struct ConnectPeerRequest {
    pub multiaddr: String,
}

pub async fn connect_peer(
    State(state): State<AppState>,
    Json(payload): Json<ConnectPeerRequest>,
) -> Result<StatusCode, AppError> {
    let addr: libp2p::Multiaddr = payload
        .multiaddr
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid multiaddr".to_string()))?;

    let (tx, rx) = oneshot::channel();
    state
        .cmd_tx
        .send(NodeCommand::ConnectPeer(addr, tx))
        .await
        .map_err(|_| AppError::InternalServerError("Failed to send command to node".to_string()))?;

    match rx.await.map_err(|_| {
        AppError::InternalServerError("Failed to receive response from node".to_string())
    })? {
        Ok(_) => Ok(StatusCode::OK),
        Err(_) => Err(AppError::InternalServerError(
            "Failed to connect to peer".to_string(),
        )),
    }
}

pub async fn discover_peers(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, AppError> {
    let (tx, rx) = oneshot::channel();
    state
        .cmd_tx
        .send(NodeCommand::DiscoverPeers(tx))
        .await
        .map_err(|_| AppError::InternalServerError("Failed to send command to node".to_string()))?;

    let count = rx.await.map_err(|_| {
        AppError::InternalServerError("Failed to receive response from node".to_string())
    })?;

    Ok(Json(
        serde_json::json!({ "status": "ok", "peers_queried": count }),
    ))
}
