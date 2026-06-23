//! POST /v1/search — query → 向量检索结果。

use std::sync::Arc;

use axum::extract::State;
use axum::Json;
use serde::Deserialize;

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct SearchReq {
    pub query: String,
    #[serde(default = "default_collection")]
    pub collection: String,
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    #[serde(default)]
    pub threshold: Option<f32>,
}

fn default_collection() -> String {
    "default".into()
}
fn default_top_k() -> usize {
    5
}

pub async fn search(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SearchReq>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let mut results = state
        .sdk
        .search(&req.query, &req.collection, req.top_k)
        .await?;
    if let Some(t) = req.threshold {
        results.retain(|r| r.score >= t);
    }
    Ok(Json(serde_json::json!({ "results": results })))
}
