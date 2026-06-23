//! POST /v1/embed — texts → embeddings（直接走 SDK 内 embedder）。

use std::sync::Arc;

use axum::extract::State;
use axum::Json;
use serde::Deserialize;

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct EmbedReq {
    pub texts: Vec<String>,
    #[serde(default)]
    pub model: Option<String>,
}

pub async fn embed(
    State(state): State<Arc<AppState>>,
    Json(req): Json<EmbedReq>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let refs: Vec<&str> = req.texts.iter().map(|s| s.as_str()).collect();
    let embeddings = state.sdk.embed_texts(&refs).await?;
    let dim = embeddings.first().map(|e| e.len()).unwrap_or(0);
    Ok(Json(
        serde_json::json!({ "embeddings": embeddings, "model": req.model, "dim": dim }),
    ))
}
