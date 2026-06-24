//! docpipe-server 入口 — axum /v1/* REST 服务。

mod config;
mod error;
mod jobs;
mod routes;
mod state;

use std::sync::Arc;

use config::Config;
use state::AppState;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let cfg = Config::from_env();
    let state = AppState::from_config(&cfg).unwrap_or_else(|e| {
        eprintln!("failed to init app state: {e}");
        std::process::exit(1);
    });
    let app = routes::router(Arc::new(state));
    let listener = tokio::net::TcpListener::bind(&cfg.bind_addr)
        .await
        .expect("bind");
    tracing::info!("docpipe-server listening on {}", cfg.bind_addr);
    axum::serve(listener, app).await.expect("serve");
}
