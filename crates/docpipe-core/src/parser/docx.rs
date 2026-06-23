//! DocxParser — docx-rs 提取段落文本 + 表格（无需 OCR，spec §7 DOCX 路径）。

use async_trait::async_trait;
use docx_rs::{
    read_docx, DocumentChild, ParagraphChild, RunChild, TableCellContent, TableChild,
    TableRowChild,
};

use super::DocParser;
use crate::error::{DocError, Result};
use crate::types::{DocFormat, OcrBackendKind, PageContent, ParseConfig, ParsedDocument, Table};

pub struct DocxParser;

/// 将段落列表拼接为换行分隔字符串（跳过空白段落）。
pub fn join_paragraphs(paras: &[String]) -> String {
    paras
        .iter()
        .filter(|p| !p.trim().is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join("\n")
}

#[async_trait]
impl DocParser for DocxParser {
    async fn parse(&self, bytes: &[u8], _config: &ParseConfig) -> Result<ParsedDocument> {
        let docx =
            read_docx(bytes).map_err(|e| DocError::Other(format!("docx read: {e}")))?;
        let (paragraphs, tables) = extract_docx_content(&docx);
        let text = join_paragraphs(&paragraphs);
        if text.trim().is_empty() && tables.is_empty() {
            return Err(DocError::ParseEmptyResult);
        }
        Ok(ParsedDocument {
            doc_id: uuid::Uuid::new_v4().to_string(),
            format: DocFormat::Docx,
            page_count: 1,
            ocr_used: false,
            backend: OcrBackendKind::TextLayer,
            pages: vec![PageContent { page_num: 1, text, blocks: vec![], tables }],
            warnings: vec![],
        })
    }

    fn supported_formats(&self) -> &[&str] {
        &["docx"]
    }
}

/// 从段落提取纯文本（Run → Text 叶节点拼接）。
fn para_text(p: &docx_rs::Paragraph) -> String {
    let mut s = String::new();
    for c in &p.children {
        if let ParagraphChild::Run(run) = c {
            for rc in &run.children {
                if let RunChild::Text(t) = rc {
                    s.push_str(&t.text);
                }
            }
        }
    }
    s
}

/// 遍历文档树，收集段落文本与表格。
/// docx-rs 0.4 的 DocumentChild 枚举区分 Paragraph / Table。
fn extract_docx_content(docx: &docx_rs::Docx) -> (Vec<String>, Vec<Table>) {
    let mut paragraphs = Vec::new();
    let mut tables = Vec::new();

    for child in &docx.document.children {
        match child {
            DocumentChild::Paragraph(p) => paragraphs.push(para_text(p)),
            DocumentChild::Table(t) => {
                let mut rows: Vec<Vec<String>> = Vec::new();
                for row in &t.rows {
                    let TableChild::TableRow(tr) = row;
                    let mut cells: Vec<String> = Vec::new();
                    for cell in &tr.cells {
                        let TableRowChild::TableCell(tc) = cell;
                        let mut cell_text = String::new();
                        for cc in &tc.children {
                            if let TableCellContent::Paragraph(p) = cc {
                                cell_text.push_str(&para_text(p));
                            }
                        }
                        cells.push(cell_text);
                    }
                    rows.push(cells);
                }
                tables.push(Table { rows });
            }
            _ => {}
        }
    }
    (paragraphs, tables)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_text_joins_paragraphs() {
        let paras = vec!["para one".to_string(), "para two".to_string()];
        assert_eq!(join_paragraphs(&paras), "para one\npara two");
    }

    #[tokio::test]
    #[ignore = "requires fixture DOCX with a table"]
    async fn docx_with_table_populates_tables() {
        use crate::parser::DocParser;
        use crate::types::ParseConfig;
        let bytes = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/with_table.docx"
        ))
        .unwrap();
        let doc = DocxParser.parse(&bytes, &ParseConfig::default()).await.unwrap();
        assert!(!doc.pages[0].tables.is_empty());
    }
}
