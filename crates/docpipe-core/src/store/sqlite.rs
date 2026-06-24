//! SqliteVecStore — rusqlite + sqlite-vec vec0 虚拟表。dim 在首次 upsert 时由向量长度确定建表。
//!
//! chunk_id 约定 "{doc_id}:{uuid}"（facade 负责前缀），delete(doc_id) 按前缀删除（spec §7 幂等）。
//!
//! # sqlite-vec 注册
//!
//! sqlite-vec 通过 `sqlite3_auto_extension` 全局注册扩展：一旦调用，后续所有新建
//! Connection 都会自动加载 vec0 虚拟表。`SqliteVecStore::register_extension()` 是幂等的
//! （使用 `std::sync::Once`），多次调用只执行一次 unsafe 注册。

use std::sync::{Mutex, Once};

use async_trait::async_trait;
use rusqlite::Connection;
use sqlite_vec::sqlite3_vec_init;

use super::VectorStore;
use crate::error::{DocError, Result};
use crate::types::{DocumentInfo, EmbeddedChunk, SearchResult};

/// 全局只注册一次 sqlite-vec 扩展。
static REGISTER_ONCE: Once = Once::new();

fn register_extension() {
    REGISTER_ONCE.call_once(|| {
        // AutoExtFn 是 SQLite auto_extension 协议要求的精确 C 函数类型，与 rusqlite bindgen
        // 生成的 sqlite3_auto_extension 参数类型完全一致。显式命名目标类型后，transmute 的
        // 目标类型在调用点可见；若 ABI 改变编译器会在此处报类型尺寸不匹配，避免通过
        // *const () 彻底擦除类型信息（原实现的问题所在）。
        // 安全：sqlite3_vec_init 在 C 侧声明为 int(*)(sqlite3*,char**,sqlite3_api_routines*)，
        // 即 AutoExtFn 的精确签名；Rust 的 extern 绑定将其简化为 fn()，需 transmute 还原。
        // Once 保证全进程只注册一次。
        type AutoExtFn = unsafe extern "C" fn(
            *mut rusqlite::ffi::sqlite3,
            *mut *mut std::ffi::c_char,
            *const rusqlite::ffi::sqlite3_api_routines,
        ) -> std::ffi::c_int;
        unsafe {
            rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute::<
                unsafe extern "C" fn(),
                AutoExtFn,
            >(sqlite3_vec_init)));
        }
    });
}

pub struct SqliteVecStore {
    conn: Mutex<Connection>,
}

impl SqliteVecStore {
    /// 打开文件数据库，并注册 sqlite-vec 扩展。
    pub fn new(path: &str) -> Result<Self> {
        register_extension();
        let conn =
            Connection::open(path).map_err(|e| DocError::VectorStoreError(format!("open: {e}")))?;
        Self::init(conn)
    }

    /// 打开内存数据库，并注册 sqlite-vec 扩展（测试专用）。
    pub fn in_memory() -> Result<Self> {
        register_extension();
        let conn = Connection::open_in_memory()
            .map_err(|e| DocError::VectorStoreError(format!("open_in_memory: {e}")))?;
        Self::init(conn)
    }

    fn init(conn: Connection) -> Result<Self> {
        // 元数据表：chunk_id ↔ text / collection / page_refs，rowid 与 vec0 同步
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS chunk_meta (
                rowid    INTEGER PRIMARY KEY AUTOINCREMENT,
                chunk_id TEXT NOT NULL,
                coll     TEXT NOT NULL,
                text     TEXT NOT NULL,
                page_refs TEXT NOT NULL,
                UNIQUE(chunk_id, coll)
            );",
        )
        .map_err(|e| DocError::VectorStoreError(format!("create chunk_meta: {e}")))?;
        // 文档注册表：按 (doc_id, collection) 主键唯一
        conn.execute(
            "CREATE TABLE IF NOT EXISTS documents (
                doc_id      TEXT NOT NULL,
                collection  TEXT NOT NULL,
                filename    TEXT,
                format      TEXT NOT NULL,
                page_count  INTEGER NOT NULL,
                chunk_count INTEGER NOT NULL,
                created_at  TEXT NOT NULL,
                PRIMARY KEY (doc_id, collection)
            )",
            [],
        )
        .map_err(|e| DocError::VectorStoreError(format!("create documents table: {e}")))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn row_to_doc(r: &rusqlite::Row) -> rusqlite::Result<DocumentInfo> {
        Ok(DocumentInfo {
            doc_id: r.get(0)?,
            collection: r.get(1)?,
            filename: r.get(2)?,
            format: r.get(3)?,
            page_count: r.get(4)?,
            chunk_count: r.get(5)?,
            created_at: r.get(6)?,
        })
    }

    /// 确保 vec0 虚拟表存在（dim 维）。幂等：CREATE VIRTUAL TABLE IF NOT EXISTS。
    /// vec0 要求 rowid 对应 chunk_meta.rowid，以便 JOIN。
    fn ensure_vec_table(conn: &Connection, dim: usize) -> Result<()> {
        conn.execute_batch(&format!(
            "CREATE VIRTUAL TABLE IF NOT EXISTS vec_chunks USING vec0(embedding float[{dim}]);"
        ))
        .map_err(|e| DocError::VectorStoreError(format!("create vec0: {e}")))?;
        Ok(())
    }
}

#[async_trait]
impl VectorStore for SqliteVecStore {
    /// 幂等 upsert：若 chunk_id + collection 已存在则先删除旧行再插入。
    async fn upsert(&self, chunks: &[EmbeddedChunk], collection: &str) -> Result<()> {
        if chunks.is_empty() {
            return Ok(());
        }
        let conn = self
            .conn
            .lock()
            .map_err(|_| DocError::VectorStoreError("mutex poisoned".into()))?;
        let dim = chunks[0].embedding.len();
        Self::ensure_vec_table(&conn, dim)?;

        for ec in chunks {
            // 查找旧行的 rowid（若存在）
            let old_rowid: Option<i64> = conn
                .query_row(
                    "SELECT rowid FROM chunk_meta WHERE chunk_id = ?1 AND coll = ?2",
                    rusqlite::params![ec.chunk.chunk_id, collection],
                    |r| r.get(0),
                )
                .ok();

            // 删除旧向量和元数据
            if let Some(rid) = old_rowid {
                conn.execute(
                    "DELETE FROM vec_chunks WHERE rowid = ?1",
                    rusqlite::params![rid],
                )
                .map_err(|e| DocError::VectorStoreError(format!("delete old vec: {e}")))?;
                conn.execute(
                    "DELETE FROM chunk_meta WHERE rowid = ?1",
                    rusqlite::params![rid],
                )
                .map_err(|e| DocError::VectorStoreError(format!("delete old meta: {e}")))?;
            }

            // 插入新元数据，获取 rowid
            let page_refs =
                serde_json::to_string(&ec.chunk.page_refs).unwrap_or_else(|_| "[]".into());
            conn.execute(
                "INSERT INTO chunk_meta (chunk_id, coll, text, page_refs) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![ec.chunk.chunk_id, collection, ec.chunk.text, page_refs],
            )
            .map_err(|e| DocError::VectorStoreError(format!("insert meta: {e}")))?;
            let rowid = conn.last_insert_rowid();

            // 插入向量（JSON 文本格式：sqlite-vec 支持 "[f32, ...]" 输入）
            let vec_json = serde_json::to_string(&ec.embedding)
                .map_err(|e| DocError::VectorStoreError(format!("serialize vec: {e}")))?;
            conn.execute(
                "INSERT INTO vec_chunks (rowid, embedding) VALUES (?1, ?2)",
                rusqlite::params![rowid, vec_json],
            )
            .map_err(|e| DocError::VectorStoreError(format!("insert vec: {e}")))?;
        }
        Ok(())
    }

    /// KNN 搜索：使用 vec0 MATCH 语法，返回 top_k 最近邻（按余弦距离）。
    async fn search(
        &self,
        query_vec: &[f32],
        collection: &str,
        top_k: usize,
    ) -> Result<Vec<SearchResult>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| DocError::VectorStoreError("mutex poisoned".into()))?;

        // vec0 KNN 查询：embedding MATCH ? 约束 + k = ?（hidden column）
        // JOIN chunk_meta 以获取 text/page_refs 并按 collection 过滤
        let vec_json = serde_json::to_string(&query_vec)
            .map_err(|e| DocError::VectorStoreError(format!("serialize query: {e}")))?;

        let mut stmt = conn
            .prepare(
                "SELECT m.chunk_id, m.text, m.page_refs, v.distance
                 FROM vec_chunks v
                 JOIN chunk_meta m ON m.rowid = v.rowid
                 WHERE v.embedding MATCH ?1
                   AND k = ?2
                   AND m.coll = ?3
                 ORDER BY v.distance",
            )
            .map_err(|e| DocError::VectorStoreError(format!("prepare search: {e}")))?;

        let rows = stmt
            .query_map(rusqlite::params![vec_json, top_k as i64, collection], |r| {
                let chunk_id: String = r.get(0)?;
                let text: String = r.get(1)?;
                let page_refs_json: String = r.get(2)?;
                let distance: f64 = r.get(3)?;
                Ok((chunk_id, text, page_refs_json, distance))
            })
            .map_err(|e| DocError::VectorStoreError(format!("search query: {e}")))?;

        let mut out = Vec::new();
        for row in rows {
            let (chunk_id, text, page_refs_json, distance) =
                row.map_err(|e| DocError::VectorStoreError(format!("row: {e}")))?;
            let page_refs: serde_json::Value =
                serde_json::from_str(&page_refs_json).unwrap_or(serde_json::Value::Null);
            // sqlite-vec vec0 默认距离度量为 L2（欧氏距离）；score = 1 - distance
            // 是单调保序变换（距离越小 score 越高），仅用于最近邻排序，并非归一化余弦相似度。
            let score = (1.0 - distance) as f32;
            out.push(SearchResult {
                chunk_id,
                text,
                score,
                metadata: serde_json::json!({ "page_refs": page_refs }),
            });
        }
        Ok(out)
    }

    /// 按 doc_id 前缀删除所有 chunk（chunk_id LIKE "{doc_id}:%"），并删除 documents 行。
    async fn delete(&self, doc_id: &str, collection: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| DocError::VectorStoreError("mutex poisoned".into()))?;
        let prefix = format!("{doc_id}:%");

        // 收集所有匹配的 rowid
        let mut stmt = conn
            .prepare("SELECT rowid FROM chunk_meta WHERE chunk_id LIKE ?1 AND coll = ?2")
            .map_err(|e| DocError::VectorStoreError(format!("prepare delete: {e}")))?;
        let rowids: Vec<i64> = stmt
            .query_map(rusqlite::params![prefix, collection], |r| r.get(0))
            .map_err(|e| DocError::VectorStoreError(format!("delete query: {e}")))?
            .collect::<rusqlite::Result<Vec<i64>>>()
            .map_err(|e| DocError::VectorStoreError(format!("delete row: {e}")))?;

        for rid in rowids {
            conn.execute(
                "DELETE FROM vec_chunks WHERE rowid = ?1",
                rusqlite::params![rid],
            )
            .map_err(|e| DocError::VectorStoreError(format!("delete vec row: {e}")))?;
            conn.execute(
                "DELETE FROM chunk_meta WHERE rowid = ?1",
                rusqlite::params![rid],
            )
            .map_err(|e| DocError::VectorStoreError(format!("delete meta row: {e}")))?;
        }

        // 同时删除文档注册表中的行
        conn.execute(
            "DELETE FROM documents WHERE doc_id=?1 AND collection=?2",
            rusqlite::params![doc_id, collection],
        )
        .map_err(|e| DocError::VectorStoreError(format!("delete document: {e}")))?;

        Ok(())
    }

    async fn register_document(&self, info: &DocumentInfo) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| DocError::VectorStoreError("lock".into()))?;
        conn.execute(
            "INSERT INTO documents (doc_id, collection, filename, format, page_count, chunk_count, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(doc_id, collection) DO UPDATE SET
                filename=excluded.filename, format=excluded.format,
                page_count=excluded.page_count, chunk_count=excluded.chunk_count,
                created_at=excluded.created_at",
            rusqlite::params![
                info.doc_id,
                info.collection,
                info.filename,
                info.format,
                info.page_count,
                info.chunk_count,
                info.created_at
            ],
        )
        .map_err(|e| DocError::VectorStoreError(format!("register_document: {e}")))?;
        Ok(())
    }

    async fn list_documents(&self, collection: &str) -> Result<Vec<DocumentInfo>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| DocError::VectorStoreError("lock".into()))?;
        let mut stmt = conn
            .prepare(
                "SELECT doc_id, collection, filename, format, page_count, chunk_count, created_at
                 FROM documents WHERE collection=?1 ORDER BY created_at DESC",
            )
            .map_err(|e| DocError::VectorStoreError(format!("list prep: {e}")))?;
        let rows = stmt
            .query_map([collection], Self::row_to_doc)
            .map_err(|e| DocError::VectorStoreError(format!("list query: {e}")))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| DocError::VectorStoreError(format!("list row: {e}")))?);
        }
        Ok(out)
    }

    async fn get_document(&self, doc_id: &str, collection: &str) -> Result<Option<DocumentInfo>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| DocError::VectorStoreError("lock".into()))?;
        let mut stmt = conn
            .prepare(
                "SELECT doc_id, collection, filename, format, page_count, chunk_count, created_at
                 FROM documents WHERE doc_id=?1 AND collection=?2",
            )
            .map_err(|e| DocError::VectorStoreError(format!("get prep: {e}")))?;
        let mut rows = stmt
            .query_map(rusqlite::params![doc_id, collection], Self::row_to_doc)
            .map_err(|e| DocError::VectorStoreError(format!("get query: {e}")))?;
        match rows.next() {
            Some(r) => {
                Ok(Some(r.map_err(|e| {
                    DocError::VectorStoreError(format!("get row: {e}"))
                })?))
            }
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::VectorStore;
    use crate::types::{Chunk, DocumentInfo, EmbeddedChunk};

    fn mk_chunk(id: &str, vec: Vec<f32>) -> EmbeddedChunk {
        EmbeddedChunk {
            chunk: Chunk {
                chunk_id: id.into(),
                text: format!("text-{id}"),
                page_refs: vec![1],
                char_offset: 0,
            },
            embedding: vec,
        }
    }

    #[tokio::test]
    async fn upsert_then_search_returns_nearest() {
        let store = SqliteVecStore::in_memory().unwrap();
        store
            .upsert(
                &[
                    mk_chunk("doc1:a", vec![1.0, 0.0, 0.0]),
                    mk_chunk("doc1:b", vec![0.0, 1.0, 0.0]),
                ],
                "default",
            )
            .await
            .unwrap();
        let results = store.search(&[0.9, 0.1, 0.0], "default", 1).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].chunk_id, "doc1:a");
    }

    #[tokio::test]
    async fn delete_by_doc_id_removes_all_its_chunks() {
        let store = SqliteVecStore::in_memory().unwrap();
        store
            .upsert(
                &[
                    mk_chunk("doc1:a", vec![1.0, 0.0, 0.0]),
                    mk_chunk("doc2:a", vec![0.0, 1.0, 0.0]),
                ],
                "default",
            )
            .await
            .unwrap();
        store.delete("doc1", "default").await.unwrap();
        let results = store.search(&[1.0, 0.0, 0.0], "default", 5).await.unwrap();
        assert!(results.iter().all(|r| !r.chunk_id.starts_with("doc1:")));
    }

    fn mk_doc(id: &str) -> DocumentInfo {
        DocumentInfo {
            doc_id: id.into(),
            collection: "default".into(),
            filename: Some(format!("{id}.pdf")),
            format: "pdf".into(),
            page_count: 2,
            chunk_count: 3,
            created_at: "2026-06-24T00:00:00Z".into(),
        }
    }

    #[tokio::test]
    async fn document_register_list_get_delete() {
        let store = SqliteVecStore::in_memory().unwrap();
        store.register_document(&mk_doc("doc1")).await.unwrap();
        store.register_document(&mk_doc("doc2")).await.unwrap();
        let docs = store.list_documents("default").await.unwrap();
        assert_eq!(docs.len(), 2);
        let one = store.get_document("doc1", "default").await.unwrap();
        assert_eq!(one.unwrap().filename.as_deref(), Some("doc1.pdf"));
        // delete removes the documents row too
        store.delete("doc1", "default").await.unwrap();
        assert!(store
            .get_document("doc1", "default")
            .await
            .unwrap()
            .is_none());
        assert_eq!(store.list_documents("default").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn register_is_idempotent_upsert() {
        let store = SqliteVecStore::in_memory().unwrap();
        store.register_document(&mk_doc("doc1")).await.unwrap();
        let mut d = mk_doc("doc1");
        d.chunk_count = 99;
        store.register_document(&d).await.unwrap();
        let docs = store.list_documents("default").await.unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].chunk_count, 99);
    }

    #[tokio::test]
    async fn reingest_same_chunk_id_is_idempotent() {
        let store = SqliteVecStore::in_memory().unwrap();
        store
            .upsert(&[mk_chunk("doc1:a", vec![1.0, 0.0, 0.0])], "default")
            .await
            .unwrap();
        store
            .upsert(&[mk_chunk("doc1:a", vec![0.0, 0.0, 1.0])], "default")
            .await
            .unwrap();
        let results = store.search(&[0.0, 0.0, 1.0], "default", 5).await.unwrap();
        let count = results.iter().filter(|r| r.chunk_id == "doc1:a").count();
        assert_eq!(count, 1); // 不重复
    }
}
