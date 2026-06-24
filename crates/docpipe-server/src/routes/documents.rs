//! 文档管理路由 —— 列表 / 详情 / 删除。

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::state::AppState;
use docpipe_core::error::DocError;
use docpipe_core::types::DocumentInfo;

#[derive(Deserialize)]
pub struct ListQuery {
    #[serde(default = "default_collection")]
    pub collection: String,
}
fn default_collection() -> String {
    "default".into()
}

#[derive(Serialize)]
pub struct ListResponse {
    pub documents: Vec<DocumentInfo>,
}

pub async fn list_documents(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ListQuery>,
) -> Result<Json<ListResponse>, ApiError> {
    let documents = state.sdk.list_documents(&q.collection).await?;
    Ok(Json(ListResponse { documents }))
}

pub async fn get_document(
    State(state): State<Arc<AppState>>,
    Path(doc_id): Path<String>,
    Query(q): Query<ListQuery>,
) -> Result<Json<DocumentInfo>, ApiError> {
    match state.sdk.get_document(&doc_id, &q.collection).await? {
        Some(d) => Ok(Json(d)),
        None => Err(ApiError(DocError::DocumentNotFound)),
    }
}

pub async fn delete_document(
    State(state): State<Arc<AppState>>,
    Path(doc_id): Path<String>,
    Query(q): Query<ListQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    // 删不存在的 doc → 404（先查存在性）
    if state
        .sdk
        .get_document(&doc_id, &q.collection)
        .await?
        .is_none()
    {
        return Err(ApiError(DocError::DocumentNotFound));
    }
    state.sdk.delete_document(&doc_id, &q.collection).await?;
    Ok(Json(
        serde_json::json!({ "deleted": true, "doc_id": doc_id }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;
    use std::sync::Arc;

    #[tokio::test]
    async fn ingest_then_list_then_delete_via_handlers() {
        let state = Arc::new(AppState::for_test());
        // ingest an HTML doc directly through the facade the handler uses
        let html = b"<html><body>\xE5\x90\x88\xE5\x90\x8C\xE3\x80\x82</body></html>";
        let r = state
            .sdk
            .ingest_file(
                html,
                Some("a.html"),
                docpipe_core::types::ParseConfig::default(),
                "default",
                "2026-06-24T00:00:00Z",
            )
            .await
            .unwrap();
        assert!(r.chunk_count >= 1);
        // list handler
        let listed = list_documents(
            axum::extract::State(state.clone()),
            axum::extract::Query(ListQuery {
                collection: "default".into(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(listed.0.documents.len(), 1);
        // delete handler
        let _ = delete_document(
            axum::extract::State(state.clone()),
            axum::extract::Path(r.doc_id.clone()),
            axum::extract::Query(ListQuery {
                collection: "default".into(),
            }),
        )
        .await
        .unwrap();
        let after = list_documents(
            axum::extract::State(state.clone()),
            axum::extract::Query(ListQuery {
                collection: "default".into(),
            }),
        )
        .await
        .unwrap();
        assert!(after.0.documents.is_empty());
    }
}
