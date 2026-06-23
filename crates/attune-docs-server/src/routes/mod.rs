//! 路由装配。

pub mod annotate;
pub mod chunk;
pub mod embed;
pub mod health;
pub mod parse;
pub mod search;

use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;

use crate::state::AppState;

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/parse", post(parse::parse))
        .route("/v1/chunk", post(chunk::chunk))
        .route("/v1/embed", post(embed::embed))
        .route("/v1/search", post(search::search))
        .route("/v1/annotate", post(annotate::annotate))
        .route("/v1/health", get(health::health))
        .with_state(state)
}
