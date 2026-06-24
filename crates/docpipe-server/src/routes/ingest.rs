//! POST /v1/ingest —— multipart 文件一步入库；config.async=true 走异步 job。

use std::sync::Arc;

use axum::extract::{Multipart, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;

use crate::error::ApiError;
use crate::state::AppState;
use docpipe_core::error::DocError;
use docpipe_core::types::ParseConfig;

#[derive(Deserialize, Default)]
pub struct IngestConfig {
    #[serde(default)]
    pub ocr: Option<bool>,
    #[serde(default)]
    pub table_structure: Option<bool>,
    #[serde(default)]
    pub max_pages: Option<u32>,
    #[serde(default)]
    pub dpi: Option<u32>,
    #[serde(default)]
    pub r#async: bool,
    #[serde(default = "default_collection")]
    pub collection: String,
}
fn default_collection() -> String {
    "default".into()
}

pub async fn ingest(
    State(state): State<Arc<AppState>>,
    mut mp: Multipart,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let mut bytes: Option<Vec<u8>> = None;
    let mut filename: Option<String> = None;
    let mut cfg = IngestConfig {
        collection: default_collection(),
        ..Default::default()
    };
    while let Some(field) = mp
        .next_field()
        .await
        .map_err(|e| ApiError(DocError::Other(format!("multipart: {e}"))))?
    {
        match field.name() {
            Some("file") => {
                filename = field.file_name().map(|s| s.to_string());
                bytes = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| ApiError(DocError::Other(format!("file read: {e}"))))?
                        .to_vec(),
                );
            }
            Some("config") => {
                let txt = field
                    .text()
                    .await
                    .map_err(|e| ApiError(DocError::Other(format!("config read: {e}"))))?;
                if !txt.trim().is_empty() {
                    cfg = serde_json::from_str(&txt)
                        .map_err(|e| ApiError(DocError::Other(format!("config json: {e}"))))?;
                }
            }
            _ => {}
        }
    }
    let bytes = bytes.ok_or_else(|| ApiError(DocError::Other("missing file field".into())))?;
    let parse_cfg = ParseConfig {
        ocr: cfg.ocr.unwrap_or(true),
        table_structure: cfg.table_structure.unwrap_or(false),
        max_pages: cfg.max_pages,
        dpi: cfg.dpi.unwrap_or(300),
    };
    let created_at = chrono::Utc::now().to_rfc3339();
    let collection = cfg.collection.clone();
    if cfg.r#async {
        // state 是 Arc<AppState>（Clone）—— async move 把所有权移入 spawned future，满足 Send+'static
        let state2 = state.clone();
        let fname = filename;
        let ca = created_at.clone();
        let job_id = state.jobs.submit(
            async move {
                state2
                    .sdk
                    .ingest_file(&bytes, fname.as_deref(), parse_cfg, &collection, &ca)
                    .await
            },
            created_at,
        );
        Ok((
            StatusCode::ACCEPTED,
            Json(serde_json::json!({ "job_id": job_id, "status": "queued" })),
        ))
    } else {
        let r = state
            .sdk
            .ingest_file(
                &bytes,
                filename.as_deref(),
                parse_cfg,
                &collection,
                &created_at,
            )
            .await?;
        let val = serde_json::to_value(r)
            .map_err(|e| ApiError(DocError::Other(format!("serialize ingest result: {e}"))))?;
        Ok((StatusCode::OK, Json(val)))
    }
}
