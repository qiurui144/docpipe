//! 向量存储层 — trait VectorStore + SqliteVecStore（sqlite-vec）/ WeaviateStore（企业版，Task 后续）。

pub mod sqlite;

pub use sqlite::SqliteVecStore;

use crate::error::Result;
use crate::types::{EmbeddedChunk, SearchResult};
use async_trait::async_trait;

#[async_trait]
pub trait VectorStore: Send + Sync {
    async fn upsert(&self, chunks: &[EmbeddedChunk], collection: &str) -> Result<()>;
    async fn search(&self, query_vec: &[f32], collection: &str, top_k: usize) -> Result<Vec<SearchResult>>;
    async fn delete(&self, doc_id: &str, collection: &str) -> Result<()>;
}
