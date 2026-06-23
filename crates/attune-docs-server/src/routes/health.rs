//! GET /v1/health — backend 就绪状态 + ram_tier（spec §5）。

use std::sync::Arc;

use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;

use crate::state::AppState;

pub async fn health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "backends": {
            "kreuzberg": "ready",
            "mineru": if state.mineru_configured { "ready" } else { "unavailable" },
            "ollama": "ready",
            "vector_store": "sqlite"
        },
        "ram_tier": state.ram_tier
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    #[tokio::test]
    async fn health_returns_ok_json() {
        let state = AppState::for_test();
        let resp = health(State(Arc::new(state))).await;
        let (parts, body) = resp.into_response().into_parts();
        assert_eq!(parts.status, StatusCode::OK);
        let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["status"], "ok");
        assert!(json["ram_tier"].is_string());
    }
}
