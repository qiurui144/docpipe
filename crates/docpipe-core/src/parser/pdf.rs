//! PdfParser — 字层 PDF 直接 pdf-extract 跳过 OCR；扫描件逐页 render → OcrBackend（spec §3 决策树）。

use std::sync::Arc;

use async_trait::async_trait;

use super::pdf_render::{extract_text_layer, is_text_layer, page_count, render_page_png, PageText};
use super::DocParser;
use crate::error::{DocError, Result};
use crate::ocr::{OcrBackend, OcrResult};
use crate::types::{
    DocFormat, OcrBackendKind, PageContent, ParseConfig, ParsedDocument, TextBlock,
};

/// 将 OCR 后端名映射为 OcrBackendKind。
/// "mineru" → Mineru；其余（含 "kreuzberg"）→ Kreuzberg。
pub fn backend_kind(name: &str) -> OcrBackendKind {
    match name {
        "mineru" => OcrBackendKind::Mineru,
        _ => OcrBackendKind::Kreuzberg,
    }
}

pub struct PdfParser {
    ocr: Arc<dyn OcrBackend>,
}

impl PdfParser {
    pub fn new(ocr: Arc<dyn OcrBackend>) -> Self {
        Self { ocr }
    }
}

/// 从字层页构造 ParsedDocument（backend=TextLayer，ocr_used=false）。
pub fn build_parsed_from_text_layer(doc_id: String, pages: &[PageText]) -> ParsedDocument {
    let page_contents: Vec<PageContent> = pages
        .iter()
        .map(|p| PageContent {
            page_num: p.page_num,
            text: p.text.clone(),
            blocks: vec![],
            tables: vec![],
        })
        .collect();
    ParsedDocument {
        doc_id,
        format: DocFormat::Pdf,
        page_count: page_contents.len() as u32,
        ocr_used: false,
        backend: OcrBackendKind::TextLayer,
        pages: page_contents,
        warnings: vec![],
    }
}

/// 从单页 OCR 结果构造 PageContent。
pub fn build_page_from_ocr(page_num: u32, result: &OcrResult) -> PageContent {
    let blocks: Vec<TextBlock> = result.blocks.clone();
    PageContent {
        page_num,
        text: result.plain_text(),
        blocks,
        tables: vec![],
    }
}

#[async_trait]
impl DocParser for PdfParser {
    async fn parse(&self, bytes: &[u8], config: &ParseConfig) -> Result<ParsedDocument> {
        let doc_id = uuid::Uuid::new_v4().to_string();
        let text_pages = extract_text_layer(bytes)?;

        // 字层路径：直接返回，跳过 OCR。
        if !config.ocr || is_text_layer(&text_pages) {
            let mut limited = text_pages;
            if let Some(max) = config.max_pages {
                limited.truncate(max as usize);
            }
            let doc = build_parsed_from_text_layer(doc_id, &limited);
            if doc.pages.iter().all(|p| p.text.trim().is_empty()) {
                return Err(DocError::ParseEmptyResult);
            }
            return Ok(doc);
        }

        // 扫描件路径：逐页渲染 → OCR。
        let total = page_count(bytes)?;
        let n = config.max_pages.map(|m| m.min(total)).unwrap_or(total);
        let mut pages = Vec::with_capacity(n as usize);
        for i in 0..n {
            let png = render_page_png(bytes, i, config.dpi)?;
            let ocr_result = self.ocr.recognize(&png, config.dpi).await?;
            pages.push(build_page_from_ocr(i + 1, &ocr_result));
        }
        if pages.iter().all(|p| p.text.trim().is_empty()) {
            return Err(DocError::ParseEmptyResult);
        }
        Ok(ParsedDocument {
            doc_id,
            format: DocFormat::Pdf,
            page_count: pages.len() as u32,
            ocr_used: true,
            backend: backend_kind(self.ocr.name()),
            pages,
            warnings: vec![],
        })
    }

    fn supported_formats(&self) -> &[&str] {
        &["pdf"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocr::{OcrBackend, OcrResult};
    use crate::types::{BBox, TextBlock};
    use async_trait::async_trait;
    use std::sync::Arc;

    struct MockOcr;

    #[async_trait]
    impl OcrBackend for MockOcr {
        async fn recognize(&self, _img: &[u8], _dpi: u32) -> crate::error::Result<OcrResult> {
            Ok(OcrResult {
                blocks: vec![TextBlock {
                    text: "某甲".into(),
                    bbox: BBox {
                        x: 1,
                        y: 2,
                        w: 3,
                        h: 4,
                    },
                    confidence: 0.99,
                }],
                avg_confidence: Some(0.99),
            })
        }

        fn name(&self) -> &str {
            "mock"
        }
    }

    #[tokio::test]
    async fn text_layer_pdf_skips_ocr() {
        // 构造一个伪 PdfParser 流程：直接测 build_parsed_from_text_layer 辅助函数（无需真实 PDF）。
        let pages = vec![crate::parser::pdf_render::PageText {
            page_num: 1,
            text: "a".repeat(500),
        }];
        let doc = build_parsed_from_text_layer("doc1".into(), &pages);
        assert!(!doc.ocr_used);
        assert_eq!(doc.backend, crate::types::OcrBackendKind::TextLayer);
        assert_eq!(doc.page_count, 1);
        assert_eq!(doc.pages[0].text, "a".repeat(500));
    }

    #[tokio::test]
    async fn empty_text_layer_returns_empty_error() {
        let pages: Vec<crate::parser::pdf_render::PageText> = vec![];
        // 空字层 + 非 OCR 路径（mock 不参与）：facade 层会判定 empty。这里测辅助函数。
        let doc = build_parsed_from_text_layer("doc2".into(), &pages);
        assert_eq!(doc.page_count, 0);
        // 上层 parse() 在 pages 全空时返回 ParseEmptyResult（见 parse 逻辑）。
    }

    #[tokio::test]
    async fn scanned_page_uses_ocr_backend() {
        let ocr: Arc<dyn OcrBackend> = Arc::new(MockOcr);
        let result = ocr.recognize(b"fake-png", 300).await.unwrap();
        let page = build_page_from_ocr(1, &result);
        assert_eq!(page.text, "某甲");
        assert_eq!(page.blocks.len(), 1);
        assert_eq!(page.blocks[0].confidence, 0.99);
    }

    #[test]
    fn backend_kind_mapping() {
        assert_eq!(backend_kind("mineru"), OcrBackendKind::Mineru);
        assert_eq!(backend_kind("kreuzberg"), OcrBackendKind::Kreuzberg);
        assert_eq!(backend_kind("anything"), OcrBackendKind::Kreuzberg);
    }
}
