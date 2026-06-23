//! 文档解析层 — trait DocParser + 各格式实现 + 格式自动检测。

pub mod auto;
pub mod pdf;
pub mod pdf_render;

use crate::error::Result;
use crate::types::{ParseConfig, ParsedDocument};
use async_trait::async_trait;

/// 根据文件头部字节判定格式。ZIP 容器（DOCX/EPUB）需读内部 marker 区分。
pub fn detect_format(bytes: &[u8]) -> Result<crate::types::DocFormat> {
    auto::detect_format(bytes)
}

/// 文档解析器 trait — 各格式实现一个（PDF/DOCX/HTML/EPUB）。
#[async_trait]
pub trait DocParser: Send + Sync {
    async fn parse(&self, bytes: &[u8], config: &ParseConfig) -> Result<ParsedDocument>;
    fn supported_formats(&self) -> &[&str];
}
