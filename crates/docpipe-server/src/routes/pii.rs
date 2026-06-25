//! POST /v1/detect-pii — PII 检测 + 可选脱敏/标注。

use std::sync::Arc;

use axum::{extract::State, http::StatusCode, Json};
use docpipe_core::pii::{self, PiiKind};
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
    let text = match (&req.text, &req.doc_id) {
        (Some(t), _) => t.clone(),
        (None, Some(id)) => match state.sdk.document_text(id, &req.collection).await {
            Ok(t) => t,
            Err(e) => {
                return (
                    StatusCode::from_u16(e.http_status()).unwrap(),
                    Json(serde_json::json!({"error": e.code()})),
                )
            }
        },
        (None, None) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({"error": "bad-request", "detail": "text or doc_id required"}),
                ),
            )
        }
    };

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

    let res = pii::detect(&text, state.ner.as_ref(), kinds.as_deref()).await;

    // 先收集所有 warnings，最后统一写入 body。
    let mut warnings = res.warnings.clone();

    // annotate=true 时跳过并 warn：doc_id 必须提供，且需要额外 annotator 集成；
    // 本版本 detect+redact 已实现，annotate 路径留待 Task 8。
    if req.annotate {
        warnings.push(
            "annotate=true not yet implemented: use POST /v1/annotate to persist entities"
                .to_string(),
        );
    }

    let mut body = serde_json::json!({ "entities": res.entities, "warnings": warnings });

    if req.redact {
        let (red, map) = pii::redact_text(&text, &res.entities);
        body["redacted_text"] = serde_json::json!(red);
        body["mapping"] = serde_json::json!(map);
    }

    (StatusCode::OK, Json(body))
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
}
