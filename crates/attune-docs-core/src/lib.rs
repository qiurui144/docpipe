//! attune-docs-core — 文档处理 SDK 核心库（解析 / OCR / 分块 / 向量化 / 存储 / 标注）。

pub mod error;
pub mod types;

#[cfg(test)]
mod scaffold_tests {
    #[test]
    fn workspace_compiles() {
        assert_eq!(2 + 2, 4);
    }
}
