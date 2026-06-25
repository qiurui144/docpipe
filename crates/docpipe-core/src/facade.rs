//! Docpipe 门面 + Builder — 装配 parser/ocr/chunker/embedder/store/annotator（spec §5 Rust API）。

use std::sync::Arc;

use crate::annotator::{create_item, AnnotateRequest};
use crate::chunker::{chunk_text, ChunkConfig};
use crate::embedder::Embedder;
use crate::error::{DocError, Result};
use crate::ocr::mineru::MinerUBackend;
use crate::ocr::OcrBackend;
use crate::parser::auto::detect_format;
use crate::parser::docx::DocxParser;
use crate::parser::html::HtmlParser;
use crate::parser::pdf::PdfParser;
use crate::parser::DocParser;
use crate::store::VectorStore;
use crate::types::{
    AnnotatableItem, ChunkLocator, DocFormat, EmbeddedChunk, ParseConfig, ParsedDocument,
    SearchResult,
};

pub struct DocpipeBuilder {
    ocr: Option<Arc<dyn OcrBackend>>,
    store: Option<Arc<dyn VectorStore>>,
    embedder: Option<Arc<dyn Embedder>>,
    mineru: Option<Arc<MinerUBackend>>,
}

impl DocpipeBuilder {
    pub fn new() -> Self {
        Self {
            ocr: None,
            store: None,
            embedder: None,
            mineru: None,
        }
    }

    pub fn ocr_backend(mut self, b: Arc<dyn OcrBackend>) -> Self {
        self.ocr = Some(b);
        self
    }

    pub fn vector_store(mut self, s: Arc<dyn VectorStore>) -> Self {
        self.store = Some(s);
        self
    }

    pub fn embedder(mut self, e: Arc<dyn Embedder>) -> Self {
        self.embedder = Some(e);
        self
    }

    pub fn mineru(mut self, m: Arc<MinerUBackend>) -> Self {
        self.mineru = Some(m);
        self
    }

    pub fn build(self) -> Result<Docpipe> {
        Ok(Docpipe {
            ocr: self
                .ocr
                .ok_or_else(|| DocError::Other("ocr_backend required".into()))?,
            store: self
                .store
                .ok_or_else(|| DocError::Other("vector_store required".into()))?,
            embedder: self
                .embedder
                .ok_or_else(|| DocError::Other("embedder required".into()))?,
            mineru: self.mineru,
        })
    }
}

impl Default for DocpipeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Docpipe {
    ocr: Arc<dyn OcrBackend>,
    store: Arc<dyn VectorStore>,
    embedder: Arc<dyn Embedder>,
    mineru: Option<Arc<MinerUBackend>>,
}

impl Docpipe {
    /// 解析文档字节：自动检测格式，路由到对应解析器。
    /// PDF + table_structure：若 MinerU 健康则用 MinerU，否则回退 kreuzberg 并附 warning。
    pub async fn parse(&self, bytes: &[u8], config: ParseConfig) -> Result<ParsedDocument> {
        let format = detect_format(bytes)?;
        match format {
            DocFormat::Pdf => {
                let mut warnings = Vec::new();
                // 选择 OCR 后端：table_structure 且 MinerU 健康 → MinerU，否则 kreuzberg。
                let ocr: Arc<dyn OcrBackend> = if config.table_structure {
                    match &self.mineru {
                        Some(m) if m.health().await => m.clone() as Arc<dyn OcrBackend>,
                        Some(_) => {
                            warnings.push("mineru-unavailable, fallback to kreuzberg".to_string());
                            self.ocr.clone()
                        }
                        None => {
                            warnings
                                .push("mineru-not-configured, fallback to kreuzberg".to_string());
                            self.ocr.clone()
                        }
                    }
                } else {
                    self.ocr.clone()
                };
                // PdfParser 内部通过 ocr.name() 推导 backend 字段，无需门面层再修正。
                let parser = PdfParser::new(ocr);
                let mut doc = parser.parse(bytes, &config).await?;
                doc.warnings.extend(warnings);
                Ok(doc)
            }
            DocFormat::Docx => DocxParser.parse(bytes, &config).await,
            DocFormat::Html => HtmlParser.parse(bytes, &config).await,
            // EPUB 解析在 v1.0 仅占位（spec §2 EPUB 后续版本支持）。
            DocFormat::Epub => Err(DocError::FormatUnsupported),
        }
    }

    /// 幂等 ingest：先删旧 doc 的所有 chunk，再分块 → 向量化 → upsert → 登记 documents 行。
    /// chunk_id 格式："{doc_id}:{uuid}"。返回 IngestResult（含 chunk_ids + 文档元数据）。
    pub async fn ingest(
        &self,
        parsed: &ParsedDocument,
        collection: &str,
        filename: Option<&str>,
        created_at: &str,
    ) -> Result<crate::types::IngestResult> {
        // 幂等：先删旧 doc 向量（同时删 documents 行）。
        self.store.delete(&parsed.doc_id, collection).await?;

        let cfg = ChunkConfig::default();
        let mut all_chunks = Vec::new();
        for page in &parsed.pages {
            let mut page_chunks = chunk_text(&page.text, &cfg);
            for c in &mut page_chunks {
                // 前缀 chunk_id，携带 page_num 到 page_refs。
                c.chunk_id = format!("{}:{}", parsed.doc_id, c.chunk_id);
                c.page_refs = vec![page.page_num];
            }
            all_chunks.extend(page_chunks);
        }
        if all_chunks.is_empty() {
            return Err(DocError::ParseEmptyResult);
        }

        let texts: Vec<&str> = all_chunks.iter().map(|c| c.text.as_str()).collect();
        let embeddings = self.embedder.embed_batch(&texts).await?;
        if embeddings.len() != all_chunks.len() {
            return Err(DocError::EmbeddingFailed(format!(
                "embedding count {} != chunk count {}",
                embeddings.len(),
                all_chunks.len()
            )));
        }

        let embedded: Vec<EmbeddedChunk> = all_chunks
            .into_iter()
            .zip(embeddings)
            .map(|(chunk, embedding)| EmbeddedChunk { chunk, embedding })
            .collect();

        let ids: Vec<String> = embedded.iter().map(|e| e.chunk.chunk_id.clone()).collect();
        self.store.upsert(&embedded, collection).await?;
        // 向量写成功后再登记文档行（不留悬空 documents 行）。
        // 若 register_document 失败，补偿性删除本次写入的向量，保持一致性（spec §11）。
        let info = crate::types::DocumentInfo {
            doc_id: parsed.doc_id.clone(),
            collection: collection.to_string(),
            filename: filename.map(|s| s.to_string()),
            format: format!("{:?}", parsed.format).to_lowercase(),
            page_count: parsed.page_count,
            chunk_count: ids.len() as u32,
            created_at: created_at.to_string(),
        };
        if let Err(e) = self.store.register_document(&info).await {
            // 尽力回滚向量写入；回滚失败记录但不覆盖原始错误。
            let _ = self.store.delete(&parsed.doc_id, collection).await;
            return Err(e);
        }
        Ok(crate::types::IngestResult {
            doc_id: parsed.doc_id.clone(),
            collection: collection.to_string(),
            chunk_count: ids.len(),
            chunk_ids: ids,
            backend: parsed.backend.clone(),
            ocr_used: parsed.ocr_used,
        })
    }

    /// 一步摄入：parse → ingest（分块+向量化+存储+登记）。
    pub async fn ingest_file(
        &self,
        bytes: &[u8],
        filename: Option<&str>,
        config: ParseConfig,
        collection: &str,
        created_at: &str,
    ) -> Result<crate::types::IngestResult> {
        let parsed = self.parse(bytes, config).await?;
        self.ingest(&parsed, collection, filename, created_at).await
    }

    pub async fn list_documents(
        &self,
        collection: &str,
    ) -> Result<Vec<crate::types::DocumentInfo>> {
        self.store.list_documents(collection).await
    }

    pub async fn get_document(
        &self,
        doc_id: &str,
        collection: &str,
    ) -> Result<Option<crate::types::DocumentInfo>> {
        self.store.get_document(doc_id, collection).await
    }

    /// 删除文档 + 其所有向量（store.delete 已同时删 documents 行）。
    pub async fn delete_document(&self, doc_id: &str, collection: &str) -> Result<()> {
        self.store.delete(doc_id, collection).await
    }

    /// 向量搜索：embed query 后在 store 中 KNN 检索。
    pub async fn search(
        &self,
        query: &str,
        collection: &str,
        top_k: usize,
    ) -> Result<Vec<SearchResult>> {
        let qv = self.embedder.embed_batch(&[query]).await?;
        let query_vec = qv
            .into_iter()
            .next()
            .ok_or_else(|| DocError::EmbeddingFailed("empty query embedding".into()))?;
        self.store.search(&query_vec, collection, top_k).await
    }

    /// 创建标注项（同步，无 IO）。
    pub fn annotate(&self, req: AnnotateRequest) -> AnnotatableItem {
        create_item(req)
    }

    /// 直接暴露底层 embedder（/v1/embed 路由用）。
    pub async fn embed_texts(&self, texts: &[&str]) -> crate::error::Result<Vec<Vec<f32>>> {
        self.embedder.embed_batch(texts).await
    }

    /// 返回文档的完整文本（各 chunk 按存储顺序拼接，`\n` 分隔）。
    /// 若 doc_id 不存在（无 chunk），返回 `DocError::DocumentNotFound`。
    pub async fn document_text(&self, doc_id: &str, collection: &str) -> Result<String> {
        let chunks = self.store.chunks_for_document(doc_id, collection).await?;
        if chunks.is_empty() {
            return Err(DocError::DocumentNotFound);
        }
        Ok(chunks.join("\n"))
    }

    /// 返回文档所有 chunk 的 (page_num, char_offset, text) 定位符（按存储顺序）。
    pub async fn document_locators(
        &self,
        doc_id: &str,
        collection: &str,
    ) -> Result<Vec<ChunkLocator>> {
        self.store.document_locators(doc_id, collection).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedder::Embedder;
    use crate::ocr::{OcrBackend, OcrResult};
    use crate::store::SqliteVecStore;
    use async_trait::async_trait;
    use std::sync::Arc;

    struct DummyOcr;
    #[async_trait]
    impl OcrBackend for DummyOcr {
        async fn recognize(&self, _i: &[u8], _d: u32) -> crate::error::Result<OcrResult> {
            Ok(OcrResult {
                blocks: vec![],
                avg_confidence: None,
            })
        }
        fn name(&self) -> &str {
            "dummy"
        }
    }

    struct DummyEmbedder;
    #[async_trait]
    impl Embedder for DummyEmbedder {
        async fn embed_batch(&self, texts: &[&str]) -> crate::error::Result<Vec<Vec<f32>>> {
            // 简单确定性向量：长度 3，第一维 = 文本字符数。
            Ok(texts
                .iter()
                .map(|t| vec![t.chars().count() as f32, 0.0, 0.0])
                .collect())
        }
        fn dim(&self) -> usize {
            3
        }
        fn model_name(&self) -> &str {
            "dummy"
        }
    }

    fn build_sdk() -> Docpipe {
        DocpipeBuilder::new()
            .ocr_backend(Arc::new(DummyOcr))
            .vector_store(Arc::new(SqliteVecStore::in_memory().unwrap()))
            .embedder(Arc::new(DummyEmbedder))
            .build()
            .unwrap()
    }

    #[test]
    fn build_fails_without_required_backends() {
        let r = DocpipeBuilder::new().build();
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn ingest_then_search_roundtrip() {
        let sdk = build_sdk();
        let parsed = crate::types::ParsedDocument {
            doc_id: "docX".into(),
            format: crate::types::DocFormat::Html,
            page_count: 1,
            ocr_used: false,
            backend: crate::types::OcrBackendKind::TextLayer,
            pages: vec![crate::types::PageContent {
                page_num: 1,
                text: "短句一。一个比较长的句子内容内容内容内容。".into(),
                blocks: vec![],
                tables: vec![],
            }],
            warnings: vec![],
        };
        let result = sdk
            .ingest(&parsed, "default", None, "2026-06-24T00:00:00Z")
            .await
            .unwrap();
        assert!(!result.chunk_ids.is_empty());
        assert!(result.chunk_ids.iter().all(|id| id.starts_with("docX:")));
        assert_eq!(result.chunk_ids.len(), result.chunk_count);
        let results = sdk
            .search("一个比较长的句子内容内容内容内容。", "default", 1)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn embed_texts_delegates_to_embedder() {
        let sdk = build_sdk();
        let vecs = sdk.embed_texts(&["ab"]).await.unwrap();
        assert_eq!(vecs[0][0], 2.0);
    }

    #[tokio::test]
    async fn reingest_same_doc_is_idempotent() {
        let sdk = build_sdk();
        let parsed = crate::types::ParsedDocument {
            doc_id: "docY".into(),
            format: crate::types::DocFormat::Html,
            page_count: 1,
            ocr_used: false,
            backend: crate::types::OcrBackendKind::TextLayer,
            pages: vec![crate::types::PageContent {
                page_num: 1,
                text: "唯一句子内容。".into(),
                blocks: vec![],
                tables: vec![],
            }],
            warnings: vec![],
        };
        sdk.ingest(&parsed, "default", None, "2026-06-24T00:00:00Z")
            .await
            .unwrap();
        sdk.ingest(&parsed, "default", None, "2026-06-24T00:00:00Z")
            .await
            .unwrap();
        let results = sdk.search("唯一句子内容。", "default", 10).await.unwrap();
        let from_docy = results
            .iter()
            .filter(|r| r.chunk_id.starts_with("docY:"))
            .count();
        assert_eq!(from_docy, 1);
    }

    /// 构建一个 SDK 实例，并将 `chunks` 作为单页文档 `doc_id` 写入 `collection`。
    async fn test_sdk_with_doc(doc_id: &str, collection: &str, chunks: &[&str]) -> Docpipe {
        let sdk = build_sdk();
        let text = chunks.join("\n");
        let parsed = crate::types::ParsedDocument {
            doc_id: doc_id.into(),
            format: crate::types::DocFormat::Html,
            page_count: 1,
            ocr_used: false,
            backend: crate::types::OcrBackendKind::TextLayer,
            pages: vec![crate::types::PageContent {
                page_num: 1,
                text,
                blocks: vec![],
                tables: vec![],
            }],
            warnings: vec![],
        };
        sdk.ingest(&parsed, collection, None, "2026-06-24T00:00:00Z")
            .await
            .unwrap();
        sdk
    }

    #[tokio::test]
    async fn document_locators_carry_page_and_offset() {
        let sdk = test_sdk_with_doc("d2", "default", &["第一段 a@b.co", "第二段 某甲"]).await;
        let locs = sdk.document_locators("d2", "default").await.unwrap();
        assert!(!locs.is_empty());
        assert!(locs.iter().all(|l| l.page_num == 1));
        assert!(locs.iter().any(|l| l.text.contains("a@b.co")));
        assert!(sdk.document_locators("missing", "default").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn document_text_concatenates_chunks() {
        let sdk = test_sdk_with_doc("d1", "default", &["第一段 a@b.co", "第二段 某甲"]).await;
        let t = sdk.document_text("d1", "default").await.unwrap();
        assert!(t.contains("a@b.co") && t.contains("某甲"));
        assert!(sdk.document_text("nope", "default").await.is_err());
    }

    #[tokio::test]
    async fn ingest_file_registers_document_and_returns_result() {
        let sdk = build_sdk();
        let html = b"<html><body>\xE5\x90\x88\xE5\x90\x8C\xE5\xAE\xA1\xE6\x9F\xA5\xE3\x80\x82</body></html>";
        let r = sdk
            .ingest_file(
                html,
                Some("c.html"),
                ParseConfig::default(),
                "default",
                "2026-06-24T00:00:00Z",
            )
            .await
            .unwrap();
        assert_eq!(r.collection, "default");
        assert!(r.chunk_count >= 1);
        assert_eq!(r.chunk_ids.len(), r.chunk_count);
        let docs = sdk.list_documents("default").await.unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].filename.as_deref(), Some("c.html"));
        assert_eq!(docs[0].doc_id, r.doc_id);
        let got = sdk.get_document(&r.doc_id, "default").await.unwrap();
        assert!(got.is_some());
        sdk.delete_document(&r.doc_id, "default").await.unwrap();
        assert!(sdk.list_documents("default").await.unwrap().is_empty());
    }
}
