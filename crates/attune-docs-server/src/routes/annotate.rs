//! POST /v1/annotate — 创建标注项。

use std::sync::Arc;

use attune_docs_core::annotator::AnnotateRequest;
use attune_docs_core::types::{AnnotationSource, BBox};
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;

use crate::state::AppState;

#[derive(Deserialize)]
pub struct AnnotateReq {
    pub doc_id: String,
    pub original_text: String,
    pub content: String,
    pub label: String,
    pub color: String,
    pub locator: LocatorReq,
    pub source: String,
    #[serde(default)]
    pub skill_metadata: Option<serde_json::Value>,
}

#[derive(Deserialize)]
pub struct LocatorReq {
    pub page_num: u32,
    pub char_offset: u32,
    #[serde(default)]
    pub bbox: Option<[u32; 4]>,
}

pub async fn annotate(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AnnotateReq>,
) -> (StatusCode, Json<serde_json::Value>) {
    // 仅接受 "ai" / "human"（忽略大小写），其他值返回 400
    let source = if req.source.eq_ignore_ascii_case("ai") {
        AnnotationSource::Ai
    } else if req.source.eq_ignore_ascii_case("human") {
        AnnotationSource::Human
    } else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "invalid-source",
                "detail": "source must be 'ai' or 'human'"
            })),
        );
    };
    let bbox = req.locator.bbox.map(|b| BBox { x: b[0], y: b[1], w: b[2], h: b[3] });
    let item = state.sdk.annotate(AnnotateRequest {
        doc_id: req.doc_id,
        original_text: req.original_text,
        content: req.content,
        label: req.label,
        color: req.color,
        page_num: req.locator.page_num,
        char_offset: req.locator.char_offset,
        bbox,
        source,
        skill_metadata: req.skill_metadata,
    });
    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "item_id": item.item_id,
            "text_hash": item.locator.text_hash
        })),
    )
}
