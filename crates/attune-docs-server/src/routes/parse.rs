//! POST /v1/parse — multipart file + JSON config → ParsedDocument。

use std::sync::Arc;

use attune_docs_core::error::DocError;
use attune_docs_core::types::{ParseConfig, ParsedDocument};
use axum::extract::{Multipart, State};
use axum::Json;

use crate::error::ApiError;
use crate::state::AppState;

pub async fn parse(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<Json<ParsedDocument>, ApiError> {
    let mut file_bytes: Option<Vec<u8>> = None;
    let mut config = ParseConfig::default();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError(DocError::Other(format!("multipart: {e}"))))?
    {
        match field.name() {
            Some("file") => {
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|e| ApiError(DocError::Other(format!("file read: {e}"))))?;
                file_bytes = Some(bytes.to_vec());
            }
            Some("config") => {
                let txt = field.text().await.unwrap_or_default();
                if !txt.trim().is_empty() {
                    config = serde_json::from_str(&txt)
                        .map_err(|e| ApiError(DocError::Other(format!("config json: {e}"))))?;
                }
            }
            _ => {}
        }
    }

    let bytes = file_bytes
        .ok_or_else(|| ApiError(DocError::Other("missing file field".into())))?;
    let doc = state.sdk.parse(&bytes, config).await?;
    Ok(Json(doc))
}
