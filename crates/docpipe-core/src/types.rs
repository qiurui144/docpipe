//! SDK 核心数据类型 — 解析结果 / 分块 / 标注，全部 serde 可序列化（HTTP 契约 spec §5）。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BBox {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TextBlock {
    pub text: String,
    pub bbox: BBox,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Table {
    pub rows: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PageContent {
    pub page_num: u32,
    pub text: String,
    pub blocks: Vec<TextBlock>,
    pub tables: Vec<Table>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DocFormat {
    Pdf,
    Docx,
    Epub,
    Html,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum OcrBackendKind {
    Kreuzberg,
    Mineru,
    TextLayer,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParsedDocument {
    pub doc_id: String,
    pub format: DocFormat,
    pub page_count: u32,
    pub ocr_used: bool,
    pub backend: OcrBackendKind,
    pub pages: Vec<PageContent>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParseConfig {
    pub ocr: bool,
    pub table_structure: bool,
    pub max_pages: Option<u32>,
    pub dpi: u32,
}

impl Default for ParseConfig {
    fn default() -> Self {
        Self {
            ocr: true,
            table_structure: false,
            max_pages: None,
            dpi: 300,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Chunk {
    pub chunk_id: String,
    pub text: String,
    pub page_refs: Vec<u32>,
    pub char_offset: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EmbeddedChunk {
    pub chunk: Chunk,
    pub embedding: Vec<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SearchResult {
    pub chunk_id: String,
    pub text: String,
    pub score: f32,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AnnotationSource {
    Ai,
    Human,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TextLocator {
    pub page_num: u32,
    pub char_offset: u32,
    pub bbox: Option<BBox>,
    pub text_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnnotatableItem {
    pub item_id: String,
    pub original_text: String,
    pub content: String,
    pub label: String,
    pub color: String,
    pub locator: TextLocator,
    pub source: AnnotationSource,
    pub skill_metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DocumentInfo {
    pub doc_id: String,
    pub collection: String,
    pub filename: Option<String>,
    pub format: String,
    pub page_count: u32,
    pub chunk_count: u32,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChunkLocator {
    pub page_num: u32,
    pub char_offset: u32,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IngestResult {
    pub doc_id: String,
    pub collection: String,
    pub chunk_count: usize,
    pub chunk_ids: Vec<String>,
    pub backend: OcrBackendKind,
    pub ocr_used: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_config_default_matches_spec() {
        let c = ParseConfig::default();
        assert!(c.ocr);
        assert!(!c.table_structure);
        assert_eq!(c.max_pages, None);
        assert_eq!(c.dpi, 300);
    }

    #[test]
    fn ocr_backend_kind_serializes_kebab() {
        let j = serde_json::to_string(&OcrBackendKind::TextLayer).unwrap();
        assert_eq!(j, "\"text-layer\"");
    }

    #[test]
    fn doc_format_serializes_lowercase() {
        let j = serde_json::to_string(&DocFormat::Pdf).unwrap();
        assert_eq!(j, "\"pdf\"");
    }

    #[test]
    fn parsed_document_roundtrips_json() {
        let doc = ParsedDocument {
            doc_id: "abc".into(),
            format: DocFormat::Pdf,
            page_count: 1,
            ocr_used: true,
            backend: OcrBackendKind::Kreuzberg,
            pages: vec![PageContent {
                page_num: 1,
                text: "hello".into(),
                blocks: vec![TextBlock {
                    text: "hello".into(),
                    bbox: BBox {
                        x: 0,
                        y: 0,
                        w: 10,
                        h: 5,
                    },
                    confidence: 0.99,
                }],
                tables: vec![],
            }],
            warnings: vec![],
        };
        let j = serde_json::to_string(&doc).unwrap();
        let back: ParsedDocument = serde_json::from_str(&j).unwrap();
        assert_eq!(doc, back);
    }
}

#[cfg(test)]
mod v11_tests {
    use super::*;

    #[test]
    fn document_info_roundtrips() {
        let d = DocumentInfo {
            doc_id: "d1".into(),
            collection: "default".into(),
            filename: Some("a.pdf".into()),
            format: "pdf".into(),
            page_count: 3,
            chunk_count: 7,
            created_at: "2026-06-24T00:00:00Z".into(),
        };
        let j = serde_json::to_string(&d).unwrap();
        assert!(j.contains("\"doc_id\":\"d1\""));
        assert_eq!(serde_json::from_str::<DocumentInfo>(&j).unwrap(), d);
    }

    #[test]
    fn ingest_result_serializes_snake_case() {
        let r = IngestResult {
            doc_id: "d1".into(),
            collection: "default".into(),
            chunk_count: 2,
            chunk_ids: vec!["d1:a".into(), "d1:b".into()],
            backend: OcrBackendKind::TextLayer,
            ocr_used: false,
        };
        let j = serde_json::to_string(&r).unwrap();
        assert!(j.contains("\"chunk_count\":2"));
        assert!(j.contains("\"backend\":\"text-layer\""));
    }
}
