//! 文本分块层 — 语义感知 sliding window，尊重句边界与标题层级。

pub mod semantic;

pub use semantic::chunk_text;

#[derive(Debug, Clone)]
pub struct ChunkConfig {
    pub chunk_size: usize, // 最大字符数（CJK token≈char 近似）
    pub overlap: f32,      // 重叠比例 0.0..1.0
    pub respect_headings: bool,
}

impl Default for ChunkConfig {
    fn default() -> Self {
        Self {
            chunk_size: 512,
            overlap: 0.2,
            respect_headings: true,
        }
    }
}
