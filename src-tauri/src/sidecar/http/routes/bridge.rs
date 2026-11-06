use crate::sidecar::{
    base_connector,
    http::{app_error::AppError, AppState},
};
use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/bridge", get(status))
        .route("/api/bridge/secret", post(save_secret))
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    connection: String,
    secret_configured: bool,
}
async fn status(State(state): State<Arc<AppState>>) -> Json<Status> {
    Json(Status {
        connection: state.bridge_status.read().await.clone(),
        secret_configured: base_connector::has_secret(),
    })
}
#[derive(Deserialize)]
struct Input {
    secret: String,
}
async fn save_secret(Json(input): Json<Input>) -> Result<Json<serde_json::Value>, AppError> {
    base_connector::store_secret(&input.secret).map_err(AppError::internal)?;
    Ok(Json(serde_json::json!({"saved": true})))
}
