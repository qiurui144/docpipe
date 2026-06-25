//! POST /v1/detect-pii — PII 检测 + 可选脱敏/标注。

use std::sync::Arc;

use axum::{extract::State, http::StatusCode, Json};
use docpipe_core::annotator::AnnotateRequest;
use docpipe_core::pii::{self, PiiKind};
use docpipe_core::types::AnnotationSource;
use serde::Deserialize;

use crate::state::AppState;

#[derive(Deserialize)]
pub struct DetectPiiReq {
    pub text: Option<String>,
    pub doc_id: Option<String>,
    #[serde(default = "default_collection")]
    pub collection: String,
    pub types: Option<Vec<String>>,
    #[serde(default)]
    pub redact: bool,
    #[serde(default)]
    pub annotate: bool,
}

fn default_collection() -> String {
    "default".into()
}

fn parse_kinds(v: &[String]) -> Result<Vec<PiiKind>, String> {
    v.iter()
        .map(|s| {
            serde_json::from_value(serde_json::Value::String(s.clone()))
                .map_err(|_| s.clone())
        })
        .collect()
}

pub async fn detect_pii(
    State(state): State<Arc<AppState>>,
    Json(req): Json<DetectPiiReq>,
) -> (StatusCode, Json<serde_json::Value>) {
    // 解析类型过滤器（在分支前，text/doc_id 两路都需要）。
    let kinds = match req.types.as_ref().map(|v| parse_kinds(v)) {
        Some(Err(bad)) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "bad-request",
                    "detail": format!("unknown pii kind: {bad}")
                })),
            )
        }
        Some(Ok(k)) => Some(k),
        None => None,
    };

    match (&req.text, &req.doc_id) {
        // ── 纯文本模式（无 doc_id）──────────────────────────────────────────
        (Some(text), _) => {
            let res = pii::detect(text, state.ner.as_ref(), kinds.as_deref()).await;
            let mut warnings = res.warnings.clone();
            if req.annotate {
                warnings.push("annotate requires doc_id".to_string());
            }
            let mut body =
                serde_json::json!({ "entities": res.entities, "warnings": warnings });
            if req.redact {
                let (red, map) = pii::redact_text(text, &res.entities);
                body["redacted_text"] = serde_json::json!(red);
                body["mapping"] = serde_json::json!(map);
            }
            (StatusCode::OK, Json(body))
        }

        // ── 文档模式（有 doc_id）────────────────────────────────────────────
        (None, Some(doc_id)) => {
            let collection = &req.collection;

            // 获取 chunk 定位符；空 vec = 文档不存在。
            let locators =
                match state.sdk.document_locators(doc_id, collection).await {
                    Ok(locs) => locs,
                    Err(e) => {
                        return (
                            StatusCode::from_u16(e.http_status()).unwrap(),
                            Json(serde_json::json!({"error": e.code()})),
                        )
                    }
                };

            if locators.is_empty() {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": "document-not-found"})),
                );
            }

            // 逐 chunk 检测一次，结果缓存供 entities 和 annotate 两处复用，
            // 避免非确定性 LLM NER 路径产出两次不一致的结果。
            let mut loc_results: Vec<(&docpipe_core::types::ChunkLocator, pii::DetectResult)> =
                Vec::with_capacity(locators.len());
            let mut warnings_set: std::collections::BTreeSet<String> =
                std::collections::BTreeSet::new();

            for loc in &locators {
                let res =
                    pii::detect(&loc.text, state.ner.as_ref(), kinds.as_deref())
                        .await;
                for w in &res.warnings {
                    warnings_set.insert(w.clone());
                }
                loc_results.push((loc, res));
            }

            // 从缓存结果构建带 page_num 的实体列表。
            // 重叠区域的同一实体会在相邻 chunk 各出现一次，(page_num, start, end, kind) 四元组
            // 相同即视为重复，保留第一次出现，避免响应和标注中各存两份。
            let mut seen_keys: std::collections::HashSet<(u32, usize, usize, String)> =
                std::collections::HashSet::new();
            let mut all_entities: Vec<serde_json::Value> = Vec::new();
            for (loc, res) in &loc_results {
                for ent in &res.entities {
                    let page_local_start = loc.char_offset as usize + ent.start;
                    let page_local_end = loc.char_offset as usize + ent.end;
                    let kind_str = serde_json::to_value(ent.kind)
                        .unwrap_or(serde_json::Value::Null)
                        .as_str()
                        .unwrap_or("unknown")
                        .to_string();
                    let key = (loc.page_num, page_local_start, page_local_end, kind_str.clone());
                    if !seen_keys.insert(key) {
                        // 重叠区域重复实体，跳过。
                        continue;
                    }
                    all_entities.push(serde_json::json!({
                        "kind":       kind_str,
                        "text":       ent.text,
                        "start":      page_local_start,
                        "end":        page_local_end,
                        "confidence": ent.confidence,
                        "source":     ent.source,
                        "page_num":   loc.page_num,
                    }));
                }
            }

            let warnings: Vec<String> = warnings_set.into_iter().collect();
            let mut body = serde_json::json!({
                "entities": all_entities,
                "warnings": warnings,
            });

            // redact=true：对拼接文本做平铺检测+脱敏（独立于 page-aware 实体）。
            if req.redact {
                let joined_text =
                    match state.sdk.document_text(doc_id, collection).await {
                        Ok(t) => t,
                        Err(e) => {
                            return (
                                StatusCode::from_u16(e.http_status()).unwrap(),
                                Json(serde_json::json!({"error": e.code()})),
                            )
                        }
                    };
                let flat_res =
                    pii::detect(&joined_text, state.ner.as_ref(), kinds.as_deref())
                        .await;
                let (red, map) = pii::redact_text(&joined_text, &flat_res.entities);
                body["redacted_text"] = serde_json::json!(red);
                body["mapping"] = serde_json::json!(map);
            }

            // annotate=true：从缓存的检测结果（与 entities 同源）为每个实体创建标注。
            // 复用同一 seen_keys 集合跳过重叠区域的重复实体，保证标注数 == dedup 后实体数。
            if req.annotate {
                let mut ann_seen: std::collections::HashSet<(u32, usize, usize, String)> =
                    std::collections::HashSet::new();
                let mut annotations: Vec<serde_json::Value> = Vec::new();
                for (loc, res) in &loc_results {
                    for ent in &res.entities {
                        let page_local_start = loc.char_offset as usize + ent.start;
                        let page_local_end = loc.char_offset as usize + ent.end;
                        let kind_str = serde_json::to_value(ent.kind)
                            .unwrap_or(serde_json::Value::Null)
                            .as_str()
                            .unwrap_or("unknown")
                            .to_string();
                        let key = (loc.page_num, page_local_start, page_local_end, kind_str.clone());
                        if !ann_seen.insert(key) {
                            continue;
                        }
                        let page_local_offset = loc.char_offset
                            + u32::try_from(ent.start).unwrap_or(u32::MAX);
                        let item = state.sdk.annotate(AnnotateRequest {
                            doc_id: doc_id.clone(),
                            original_text: ent.text.clone(),
                            content: format!("检测到 PII: {kind_str}"),
                            label: format!("pii-{kind_str}"),
                            color: "#ef4444".to_string(),
                            page_num: loc.page_num,
                            char_offset: page_local_offset,
                            bbox: None,
                            source: AnnotationSource::Ai,
                            skill_metadata: None,
                        });
                        annotations
                            .push(serde_json::json!({ "item_id": item.item_id }));
                    }
                }
                body["annotations"] = serde_json::json!(annotations);
            }

            (StatusCode::OK, Json(body))
        }

        // ── 两者皆无 ────────────────────────────────────────────────────────
        (None, None) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "bad-request",
                "detail": "text or doc_id required",
            })),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt as _;

    fn make_state() -> Arc<AppState> {
        Arc::new(AppState::for_test())
    }

    #[tokio::test]
    async fn detect_pii_email_found() {
        let state = make_state();
        let app = crate::routes::router(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/detect-pii")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"text":"contact a@b.co for info"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let entities = json["entities"].as_array().unwrap();
        assert!(
            entities.iter().any(|e| e["kind"] == "email"),
            "expected email entity, got: {json}"
        );
    }

    #[tokio::test]
    async fn detect_pii_unknown_type_returns_400() {
        let state = make_state();
        let app = crate::routes::router(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/detect-pii")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"text":"a@b.co","types":["bogus"]}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["error"], "bad-request", "got: {json}");
        assert!(
            json["detail"].as_str().unwrap_or("").contains("bogus"),
            "detail should name the bad kind, got: {json}"
        );
    }

    #[tokio::test]
    async fn detect_pii_missing_text_and_doc_id_returns_400() {
        let state = make_state();
        let app = crate::routes::router(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/detect-pii")
            .header("content-type", "application/json")
            .body(Body::from(r#"{}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["error"], "bad-request", "got: {json}");
    }

    /// Task 16: 文档模式 + annotate=true → 标注持久化 + page_num 携带。
    #[tokio::test]
    async fn detect_pii_annotate_doc_id() {
        let state = Arc::new(AppState::for_test());

        // 注入一个含邮箱的合成 HTML 文档。
        let html = b"<html><body>\xe8\x81\x94\xe7\xb3\xbb a@b.co</body></html>";
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

        // 调用 handler：doc_id + annotate=true。
        let req_body: DetectPiiReq = serde_json::from_value(serde_json::json!({
            "doc_id": r.doc_id,
            "annotate": true
        }))
        .unwrap();
        let (status, body) =
            detect_pii(axum::extract::State(state.clone()), axum::Json(req_body))
                .await;

        assert_eq!(status, StatusCode::OK, "expected 200, got body: {}", body.0);

        // entities 非空，每个都有 page_num。
        let entities = body.0["entities"]
            .as_array()
            .expect("entities should be an array");
        assert!(
            !entities.is_empty(),
            "expected at least one entity in doc mode, body: {}",
            body.0
        );
        for ent in entities {
            assert!(
                ent["page_num"].is_number(),
                "each entity must have page_num, got: {ent}"
            );
        }

        // annotations 非空。
        let annotations = body.0["annotations"]
            .as_array()
            .expect("annotations should be present when annotate=true");
        assert!(
            !annotations.is_empty(),
            "expected annotations non-empty, body: {}",
            body.0
        );
        for ann in annotations {
            assert!(
                ann["item_id"].is_string(),
                "annotation must have item_id string, got: {ann}"
            );
        }
    }

    /// 文档模式重叠 chunk 去重：entities 中不得有 (page_num, start, end, kind) 完全相同的两条；
    /// annotate=true 时标注数必须等于 dedup 后实体数（不因 overlap 导致重复持久化）。
    #[tokio::test]
    async fn detect_pii_doc_mode_dedups_overlapping_chunks() {
        let state = Arc::new(AppState::for_test());

        // 构造一份 HTML：正文超过 512 字符（默认 chunk_size），确保被切成 ≥2 个有重叠的 chunk。
        // 将 dup@b.co 放在正文中部，使其大概率落入相邻 chunk 的重叠区域。
        // 即使 chunk 边界未精确命中重叠区，通用不变式（无重复四元组）同样有效。
        let filler_a = "甲 ".repeat(120); // ~240 字节 UTF-8
        let filler_b = "乙 ".repeat(120);
        let body_text = format!(
            "{filler_a}联系 dup@b.co 获取支持。{filler_b}"
        );
        let html = format!("<html><body>{body_text}</body></html>");

        let r = state
            .sdk
            .ingest_file(
                html.as_bytes(),
                Some("overlap.html"),
                docpipe_core::types::ParseConfig::default(),
                "default",
                "2026-06-25T00:00:00Z",
            )
            .await
            .unwrap();

        // doc_id 模式 + annotate=true
        let req_body: DetectPiiReq = serde_json::from_value(serde_json::json!({
            "doc_id": r.doc_id,
            "annotate": true
        }))
        .unwrap();
        let (status, body) =
            detect_pii(axum::extract::State(state.clone()), axum::Json(req_body))
                .await;

        assert_eq!(status, StatusCode::OK, "expected 200, body: {}", body.0);

        let entities = body.0["entities"]
            .as_array()
            .expect("entities must be an array");

        // 通用不变式：不得有两条 (page_num, start, end, kind) 完全相同的实体。
        let mut seen: std::collections::HashSet<(u64, u64, u64, String)> =
            std::collections::HashSet::new();
        for ent in entities {
            let key = (
                ent["page_num"].as_u64().unwrap_or(0),
                ent["start"].as_u64().unwrap_or(0),
                ent["end"].as_u64().unwrap_or(0),
                ent["kind"].as_str().unwrap_or("").to_string(),
            );
            assert!(
                seen.insert(key.clone()),
                "duplicate entity (page_num={}, start={}, end={}, kind={}) in response",
                key.0, key.1, key.2, key.3
            );
        }

        // 标注数必须等于 dedup 后实体数（无重复持久化）。
        if let Some(annotations) = body.0["annotations"].as_array() {
            assert_eq!(
                annotations.len(),
                entities.len(),
                "annotations.len() should equal deduped entities.len(), \
                 but got {} annotations for {} entities; body: {}",
                annotations.len(),
                entities.len(),
                body.0
            );
        }
    }

    /// Task 16: 纯文本模式 + annotate=true → 200 + warning 含 "annotate requires doc_id"。
    #[tokio::test]
    async fn detect_pii_annotate_text_warns_no_doc_id() {
        let state = make_state();
        let req_body: DetectPiiReq = serde_json::from_value(serde_json::json!({
            "text": "a@b.co",
            "annotate": true
        }))
        .unwrap();
        let (status, body) =
            detect_pii(axum::extract::State(state.clone()), axum::Json(req_body))
                .await;

        assert_eq!(status, StatusCode::OK, "expected 200, body: {}", body.0);
        let warnings = body.0["warnings"]
            .as_array()
            .expect("warnings must be array");
        assert!(
            warnings
                .iter()
                .any(|w| w.as_str().unwrap_or("").contains("annotate requires doc_id")),
            "expected warning about annotate requires doc_id, got: {}",
            body.0
        );
    }
}
