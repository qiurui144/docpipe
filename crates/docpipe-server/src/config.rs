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
            bind_addr: env_first(&["BIND_ADDR", "DOCPIPE_LISTEN"])
                .unwrap_or_else(|| "0.0.0.0:8200".into()),
            ollama_url: std::env::var("OLLAMA_URL")
                .unwrap_or_else(|_| "http://localhost:11434".into()),
            embed_model: std::env::var("EMBED_MODEL").unwrap_or_else(|_| "bge-m3".into()),
            sqlite_path: env_first(&["SQLITE_PATH"])
                .or_else(|| {
                    database_url_to_sqlite_path(std::env::var("DATABASE_URL").ok()?.as_str())
                })
                .unwrap_or_else(|| "./docpipe.db".into()),
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

fn env_first(names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| std::env::var(name).ok())
}

fn database_url_to_sqlite_path(url: &str) -> Option<String> {
    url.strip_prefix("sqlite:///")
        .map(|p| format!("/{p}"))
        .or_else(|| url.strip_prefix("sqlite://").map(str::to_string))
        .or_else(|| url.strip_prefix("sqlite:").map(str::to_string))
        .filter(|p| !p.is_empty())
}

#[cfg(test)]
mod tests {
    use super::database_url_to_sqlite_path;

    #[test]
    fn parses_sqlite_database_url_variants() {
        assert_eq!(
            database_url_to_sqlite_path("sqlite:///tmp/docpipe.db").as_deref(),
            Some("/tmp/docpipe.db")
        );
        assert_eq!(
            database_url_to_sqlite_path("sqlite://relative.db").as_deref(),
            Some("relative.db")
        );
        assert_eq!(
            database_url_to_sqlite_path("sqlite:local.db").as_deref(),
            Some("local.db")
        );
        assert_eq!(database_url_to_sqlite_path("postgres://x"), None);
    }
}
