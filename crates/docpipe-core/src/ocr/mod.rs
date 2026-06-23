//! OCR 后端 trait — KreuzbergBackend（默认，PP-OCRv4 ONNX）/ MinerUBackend（HTTP sidecar）。

pub mod kreuzberg;
pub mod mineru;
pub mod models;

use crate::error::Result;
use crate::types::TextBlock;
use async_trait::async_trait;

/// 单页 OCR 结果 — 带坐标文字块 + 文档级平均置信度。
#[derive(Debug, Clone)]
pub struct OcrResult {
    pub blocks: Vec<TextBlock>,
    pub avg_confidence: Option<f32>,
}

impl OcrResult {
    /// 拼接所有非空文字块为纯文本（已按 reading order 排序）。
    pub fn plain_text(&self) -> String {
        self.blocks
            .iter()
            .filter(|b| !b.text.is_empty())
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// OCR 后端 trait。`recognize` 输入单页渲染图（PNG/JPEG bytes），返回带坐标文字块。
#[async_trait]
pub trait OcrBackend: Send + Sync {
    async fn recognize(&self, page_image: &[u8], dpi: u32) -> Result<OcrResult>;
    fn name(&self) -> &str;
    fn requires_gpu(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::BBox;

    #[test]
    fn plain_text_joins_blocks_with_newline() {
        let r = OcrResult {
            blocks: vec![
                TextBlock {
                    text: "line one".into(),
                    bbox: BBox {
                        x: 0,
                        y: 0,
                        w: 1,
                        h: 1,
                    },
                    confidence: 0.9,
                },
                TextBlock {
                    text: "line two".into(),
                    bbox: BBox {
                        x: 0,
                        y: 2,
                        w: 1,
                        h: 1,
                    },
                    confidence: 0.8,
                },
            ],
            avg_confidence: Some(0.85),
        };
        assert_eq!(r.plain_text(), "line one\nline two");
    }

    #[test]
    fn plain_text_skips_empty_blocks() {
        let r = OcrResult {
            blocks: vec![
                TextBlock {
                    text: "a".into(),
                    bbox: BBox {
                        x: 0,
                        y: 0,
                        w: 1,
                        h: 1,
                    },
                    confidence: 0.9,
                },
                TextBlock {
                    text: "".into(),
                    bbox: BBox {
                        x: 0,
                        y: 1,
                        w: 1,
                        h: 1,
                    },
                    confidence: 0.0,
                },
                TextBlock {
                    text: "b".into(),
                    bbox: BBox {
                        x: 0,
                        y: 2,
                        w: 1,
                        h: 1,
                    },
                    confidence: 0.9,
                },
            ],
            avg_confidence: None,
        };
        assert_eq!(r.plain_text(), "a\nb");
    }
}
