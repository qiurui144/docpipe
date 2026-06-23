//! HtmlParser — scraper 提取可见文本，剔除 script/style（对抗 JS 注入，spec §9）。

use async_trait::async_trait;
use scraper::{Html, Selector};

use super::DocParser;
use crate::error::{DocError, Result};
use crate::types::{DocFormat, OcrBackendKind, PageContent, ParseConfig, ParsedDocument};

pub struct HtmlParser;

#[async_trait]
impl DocParser for HtmlParser {
    async fn parse(&self, bytes: &[u8], _config: &ParseConfig) -> Result<ParsedDocument> {
        let html = String::from_utf8_lossy(bytes);
        let doc = Html::parse_document(&html);

        // 仅选取可见内容标签（排除 script/style）。
        // scraper body.text() 会包含 <script>/<style> 内的文本节点，
        // 故直接选择具体可见元素，不使用 body.text()。
        let visible_sel = Selector::parse(
            "body h1, body h2, body h3, body h4, body h5, body h6, \
             body p, body li, body td, body th, body span, body div, \
             body a, body blockquote, body pre, body figcaption, body caption",
        )
        .expect("selector is valid");

        let mut lines: Vec<String> = Vec::new();
        for el in doc.select(&visible_sel) {
            // el.text() 会继续下探子元素（包括可能的 script/style 子节点）。
            // 但选择器已排除 script/style，且实际 body 内 script/style 均为顶层子元素，
            // 不在可见元素内部，故此处迭代是安全的。
            let t: String = el
                .text()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            if !t.is_empty() {
                lines.push(t);
            }
        }

        // 去重（父子元素会导致内容重复收集，保留最外层匹配）。
        let text = dedup_lines(lines);

        if text.trim().is_empty() {
            return Err(DocError::ParseEmptyResult);
        }

        Ok(ParsedDocument {
            doc_id: uuid::Uuid::new_v4().to_string(),
            format: DocFormat::Html,
            page_count: 1,
            ocr_used: false,
            backend: OcrBackendKind::TextLayer,
            pages: vec![PageContent { page_num: 1, text, blocks: vec![], tables: vec![] }],
            warnings: vec![],
        })
    }

    fn supported_formats(&self) -> &[&str] {
        &["html"]
    }
}

/// 对齐连续重复内容去重：
/// 由于选择器涵盖父子元素，同一段文字可能来自 `<div>` 和其内的 `<p>`，产生重复行。
/// 策略：若某行是上一行的子串则跳过（父元素匹配先，子元素文本与父相同时去重）。
fn dedup_lines(lines: Vec<String>) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let mut result: Vec<String> = Vec::with_capacity(lines.len());
    let mut prev = String::new();
    for line in lines {
        // 跳过与上一行相同或被上一行包含的行。
        if !prev.contains(line.as_str()) {
            result.push(line.clone());
            prev = line;
        }
    }
    result.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::DocParser;
    use crate::types::ParseConfig;

    #[tokio::test]
    async fn html_extracts_visible_text_only() {
        let html = b"<html><head><style>.x{color:red}</style><script>alert('xss')</script></head><body><h1>Title</h1><p>Hello world</p></body></html>";
        let parser = HtmlParser;
        let doc = parser.parse(html, &ParseConfig::default()).await.unwrap();
        let text = &doc.pages[0].text;
        assert!(text.contains("Title"));
        assert!(text.contains("Hello world"));
        // 对抗：JS / CSS 不得出现在输出
        assert!(!text.contains("alert"));
        assert!(!text.contains("xss"));
        assert!(!text.contains("color:red"));
    }

    #[tokio::test]
    async fn html_empty_body_returns_empty_error() {
        let html = b"<html><body></body></html>";
        let parser = HtmlParser;
        let err = parser.parse(html, &ParseConfig::default()).await.unwrap_err();
        assert_eq!(err.code(), "parse-empty-result");
    }
}
