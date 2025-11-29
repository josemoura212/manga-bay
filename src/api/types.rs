use crate::app::NodeCommand;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use manga_bay_storage::Storage;
use serde_json::json;
use tokio::sync::mpsc;

#[derive(Clone)]
pub struct AppState {
    pub storage: Storage,
    pub cmd_tx: mpsc::Sender<NodeCommand>,
}

// Standardized Error Response
pub enum AppError {
    InternalServerError(String),
    BadRequest(String),
    NotFound(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            AppError::InternalServerError(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
        };

        let body = Json(json!({
            "status": "error",
            "message": message
        }));

        (status, body).into_response()
    }
}
