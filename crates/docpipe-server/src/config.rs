//! 服务配置 — 全部从环境变量读取（spec §4 config from env）。

#[derive(Clone, Debug)]
pub struct Config {
    pub bind_addr: String,
    pub ollama_url: String,
    pub embed_model: String,
    pub sqlite_path: String,
    pub mineru_url: Option<String>,
    #[allow(dead_code)] // 保留供将来 OCR 并发控制使用（spec §8 MAX_OCR_CONCURRENCY）
    pub max_ocr_concurrency: usize,
    pub max_upload_bytes: usize,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            bind_addr: std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8200".into()),
            ollama_url: std::env::var("OLLAMA_URL")
                .unwrap_or_else(|_| "http://localhost:11434".into()),
            embed_model: std::env::var("EMBED_MODEL").unwrap_or_else(|_| "bge-m3".into()),
            sqlite_path: std::env::var("SQLITE_PATH").unwrap_or_else(|_| "./docpipe.db".into()),
            mineru_url: std::env::var("MINERU_URL").ok(),
            max_ocr_concurrency: std::env::var("MAX_OCR_CONCURRENCY")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(2),
            max_upload_bytes: std::env::var("MAX_UPLOAD_BYTES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(500 * 1024 * 1024),
        }
    }
}
