//! 向量存储层 — trait VectorStore + SqliteVecStore（sqlite-vec）/ WeaviateStore（企业版，Task 后续）。

pub mod sqlite;

pub use sqlite::SqliteVecStore;

use crate::error::Result;
use crate::types::{DocumentInfo, EmbeddedChunk, SearchResult};
use async_trait::async_trait;

#[async_trait]
pub trait VectorStore: Send + Sync {
    async fn upsert(&self, chunks: &[EmbeddedChunk], collection: &str) -> Result<()>;
    async fn search(
        &self,
        query_vec: &[f32],
        collection: &str,
        top_k: usize,
    ) -> Result<Vec<SearchResult>>;
    async fn delete(&self, doc_id: &str, collection: &str) -> Result<()>;
    async fn register_document(&self, info: &DocumentInfo) -> Result<()>;
    async fn list_documents(&self, collection: &str) -> Result<Vec<DocumentInfo>>;
    async fn get_document(&self, doc_id: &str, collection: &str) -> Result<Option<DocumentInfo>>;
}
