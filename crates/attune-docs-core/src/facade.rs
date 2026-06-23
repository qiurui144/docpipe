//! AttuneDocs 门面 + Builder — 装配 parser/ocr/chunker/embedder/store/annotator（spec §5 Rust API）。

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
    AnnotatableItem, DocFormat, EmbeddedChunk, ParseConfig, ParsedDocument, SearchResult,
};

pub struct AttuneDocsBuilder {
    ocr: Option<Arc<dyn OcrBackend>>,
    store: Option<Arc<dyn VectorStore>>,
    embedder: Option<Arc<dyn Embedder>>,
    mineru: Option<Arc<MinerUBackend>>,
}

impl AttuneDocsBuilder {
    pub fn new() -> Self {
        Self { ocr: None, store: None, embedder: None, mineru: None }
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

    pub fn build(self) -> Result<AttuneDocs> {
        Ok(AttuneDocs {
            ocr: self.ocr.ok_or_else(|| DocError::Other("ocr_backend required".into()))?,
            store: self.store.ok_or_else(|| DocError::Other("vector_store required".into()))?,
            embedder: self.embedder.ok_or_else(|| DocError::Other("embedder required".into()))?,
            mineru: self.mineru,
        })
    }
}

impl Default for AttuneDocsBuilder {
    fn default() -> Self {
        Self::new()
    }
}

pub struct AttuneDocs {
    ocr: Arc<dyn OcrBackend>,
    store: Arc<dyn VectorStore>,
    embedder: Arc<dyn Embedder>,
    mineru: Option<Arc<MinerUBackend>>,
}

impl AttuneDocs {
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
                            warnings.push(
                                "mineru-not-configured, fallback to kreuzberg".to_string(),
                            );
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

    /// 幂等 ingest：先删旧 doc 的所有 chunk，再分块 → 向量化 → upsert。
    /// chunk_id 格式："{doc_id}:{uuid}"。返回已写入的 chunk_id 列表。
    pub async fn ingest(
        &self,
        parsed: &ParsedDocument,
        collection: &str,
    ) -> Result<Vec<String>> {
        // 幂等：先删旧 doc 向量。
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
        Ok(ids)
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
            Ok(OcrResult { blocks: vec![], avg_confidence: None })
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
            Ok(texts.iter().map(|t| vec![t.chars().count() as f32, 0.0, 0.0]).collect())
        }
        fn dim(&self) -> usize {
            3
        }
        fn model_name(&self) -> &str {
            "dummy"
        }
    }

    fn build_sdk() -> AttuneDocs {
        AttuneDocsBuilder::new()
            .ocr_backend(Arc::new(DummyOcr))
            .vector_store(Arc::new(SqliteVecStore::in_memory().unwrap()))
            .embedder(Arc::new(DummyEmbedder))
            .build()
            .unwrap()
    }

    #[test]
    fn build_fails_without_required_backends() {
        let r = AttuneDocsBuilder::new().build();
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
        let ids = sdk.ingest(&parsed, "default").await.unwrap();
        assert!(!ids.is_empty());
        assert!(ids.iter().all(|id| id.starts_with("docX:")));
        let results = sdk.search("一个比较长的句子内容内容内容内容。", "default", 1).await.unwrap();
        assert_eq!(results.len(), 1);
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
        sdk.ingest(&parsed, "default").await.unwrap();
        sdk.ingest(&parsed, "default").await.unwrap();
        let results = sdk.search("唯一句子内容。", "default", 10).await.unwrap();
        let from_docy = results.iter().filter(|r| r.chunk_id.starts_with("docY:")).count();
        assert_eq!(from_docy, 1);
    }
}
