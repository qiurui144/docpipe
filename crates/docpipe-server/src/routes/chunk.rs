//! POST /v1/chunk — text → chunks（spec §5）。

use axum::Json;
use docpipe_core::chunker::{chunk_text, ChunkConfig};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct ChunkReq {
    pub text: String,
    #[serde(default = "default_chunk_size")]
    pub chunk_size: usize,
    #[serde(default = "default_overlap")]
    pub overlap: f32,
    #[serde(default = "default_respect")]
    pub respect_headings: bool,
}

fn default_chunk_size() -> usize {
    512
}
fn default_overlap() -> f32 {
    0.2
}
fn default_respect() -> bool {
    true
}

pub async fn chunk(Json(req): Json<ChunkReq>) -> Json<serde_json::Value> {
    let cfg = ChunkConfig {
        chunk_size: req.chunk_size,
        overlap: req.overlap,
        respect_headings: req.respect_headings,
    };
    let chunks = chunk_text(&req.text, &cfg);
    Json(serde_json::json!({ "chunks": chunks }))
}
