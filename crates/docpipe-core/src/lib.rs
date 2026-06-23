//! docpipe-core — 文档处理 SDK 核心库（解析 / OCR / 分块 / 向量化 / 存储 / 标注）。

pub mod error;
pub mod types;
pub mod parser;
pub mod ocr;
pub mod chunker;
pub mod embedder;
pub mod store;
pub mod annotator;
pub mod facade;

pub use facade::{Docpipe, DocpipeBuilder};

#[cfg(test)]
mod scaffold_tests {
    #[test]
    fn workspace_compiles() {
        assert_eq!(2 + 2, 4);
    }
}
