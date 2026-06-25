//! 向量存储层 — trait VectorStore + SqliteVecStore（sqlite-vec）/ WeaviateStore（企业版，Task 后续）。

pub mod sqlite;

pub use sqlite::SqliteVecStore;

use crate::error::Result;
use crate::types::{ChunkLocator, DocumentInfo, EmbeddedChunk, SearchResult};
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
    /// 按插入顺序返回该文档所有 chunk 的文本（chunk_id LIKE "{doc_id}:%"）。
    async fn chunks_for_document(&self, doc_id: &str, collection: &str) -> Result<Vec<String>>;
    /// 返回文档所有 chunk 的页码 + 页内字符偏移量（按存储顺序）。
    async fn document_locators(
        &self,
        doc_id: &str,
        collection: &str,
    ) -> Result<Vec<ChunkLocator>>;
}
