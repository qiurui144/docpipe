//! 向量化层 — trait Embedder + OllamaEmbedder（HTTP /api/embed）。

pub mod ollama;

pub use ollama::OllamaEmbedder;

use crate::error::Result;
use async_trait::async_trait;

#[async_trait]
pub trait Embedder: Send + Sync {
    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>>;
    fn dim(&self) -> usize;
    fn model_name(&self) -> &str;
}
