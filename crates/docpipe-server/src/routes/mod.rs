//! 路由装配。

pub mod annotate;
pub mod chunk;
pub mod documents;
pub mod embed;
pub mod health;
pub mod ingest;
pub mod jobs;
pub mod parse;
pub mod search;

use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};
use axum::Router;

use crate::state::AppState;

pub fn router(state: Arc<AppState>) -> Router {
    // max_upload_bytes を先に読んでおく（with_state が state を move する前に）
    let limit = state.max_upload_bytes;
    Router::new()
        .route("/v1/parse", post(parse::parse))
        .route("/v1/chunk", post(chunk::chunk))
        .route("/v1/embed", post(embed::embed))
        .route("/v1/search", post(search::search))
        .route("/v1/annotate", post(annotate::annotate))
        .route("/v1/health", get(health::health))
        .route("/v1/ingest", post(ingest::ingest))
        .route("/v1/documents", get(documents::list_documents))
        .route(
            "/v1/documents/{doc_id}",
            get(documents::get_document).delete(documents::delete_document),
        )
        .route("/v1/jobs/{job_id}", get(jobs::get_job))
        .layer(DefaultBodyLimit::max(limit))
        .with_state(state)
}
