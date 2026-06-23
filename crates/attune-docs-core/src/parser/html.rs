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

        // 收集所有 script/style 中的文本节点，用于后续过滤。
        // scraper 的 body.text() 是叶子节点迭代器（不重复父子），
        // 但 <script>/<style> 中的文本节点也会被包含，需主动排除。
        let script_style_sel = Selector::parse("script, style").expect("static selector");
        let mut bad: std::collections::HashSet<String> = std::collections::HashSet::new();
        for el in doc.select(&script_style_sel) {
            for t in el.text() {
                let t = t.trim();
                if !t.is_empty() {
                    bad.insert(t.to_string());
                }
            }
        }

        // 遍历 body 所有叶子文本节点，跳过 script/style 内容。
        // body.text() 只产生叶子节点，不存在父子重复问题，无需去重。
        let body_sel = Selector::parse("body").expect("static selector");
        let mut lines: Vec<String> = Vec::new();
        if let Some(body) = doc.select(&body_sel).next() {
            for t in body.text() {
                let t = t.trim();
                if !t.is_empty() && !bad.contains(t) {
                    lines.push(t.to_string());
                }
            }
        }

        let text = lines.join("\n");

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

    /// 回归测试：旧白名单方案会丢弃 <strong>/<em>/<section> 等标签内的文本，
    /// 新方案遍历所有 body 叶子节点，确保这些内容不再丢失。
    #[tokio::test]
    async fn html_extracts_text_from_inline_and_sectioning_tags() {
        let html = b"<html><body><strong>\xe8\xad\xa6\xe5\x91\x8a\xef\xbc\x9a</strong>\xe4\xb8\x8d\xe8\xa6\x81\xe7\xbb\xa7\xe7\xbb\xad\xe3\x80\x82<section>\xe6\xad\xa3\xe6\x96\x87\xe6\xae\xb5\xe8\x90\xbd</section></body></html>";
        let parser = HtmlParser;
        let doc = parser.parse(html, &ParseConfig::default()).await.unwrap();
        let text = &doc.pages[0].text;
        // 旧白名单会丢弃 <strong> 和 <section> 内的文本
        assert!(text.contains("警告："), "应包含 <strong> 内的文本");
        assert!(text.contains("不要继续。"), "应包含 <strong> 后裸文本节点");
        assert!(text.contains("正文段落"), "应包含 <section> 内的文本");
    }
}
