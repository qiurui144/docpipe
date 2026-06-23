//! OllamaEmbedder — POST {base_url}/api/embed，3 次重试带退避（spec §3 retry=3 backoff）。

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use super::Embedder;
use crate::error::{DocError, Result};

pub struct OllamaEmbedder {
    base_url: String,
    model: String,
    dim: usize,
    client: reqwest::Client,
}

#[derive(Deserialize)]
struct EmbedResp {
    embeddings: Vec<Vec<f32>>,
}

impl OllamaEmbedder {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            model: model.into(),
            dim: 1024, // bge-m3 默认；可用 with_dim 覆盖
            client: reqwest::Client::new(),
        }
    }

    pub fn with_dim(mut self, dim: usize) -> Self {
        self.dim = dim;
        self
    }
}

#[async_trait]
impl Embedder for OllamaEmbedder {
    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let payload = json!({ "model": self.model, "input": texts });
        let mut last_err = String::new();
        for attempt in 0..3u32 {
            let resp = self
                .client
                .post(format!("{}/api/embed", self.base_url))
                .timeout(Duration::from_secs(60))
                .json(&payload)
                .send()
                .await;
            match resp {
                Ok(r) if r.status().is_success() => {
                    let parsed: EmbedResp = r
                        .json()
                        .await
                        .map_err(|e| DocError::EmbeddingFailed(format!("json: {e}")))?;
                    return Ok(parsed.embeddings);
                }
                Ok(r) => last_err = format!("status {}", r.status()),
                Err(e) => last_err = format!("request: {e}"),
            }
            // 退避：100ms, 200ms（仅在还有后续重试时等待）。
            if attempt < 2 {
                let backoff = 100u64 * 2u64.pow(attempt);
                tokio::time::sleep(Duration::from_millis(backoff)).await;
            }
        }
        Err(DocError::EmbeddingFailed(last_err))
    }

    fn dim(&self) -> usize {
        self.dim
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedder::Embedder;

    #[tokio::test]
    async fn embed_batch_parses_embeddings() {
        let mut server = mockito::Server::new_async().await;
        let body = r#"{"embeddings":[[0.1,0.2,0.3],[0.4,0.5,0.6]]}"#;
        let _m = server
            .mock("POST", "/api/embed")
            .with_status(200)
            .with_body(body)
            .create_async()
            .await;
        let emb = OllamaEmbedder::new(server.url(), "bge-m3");
        let out = emb.embed_batch(&["a", "b"]).await.unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], vec![0.1, 0.2, 0.3]);
    }

    #[tokio::test]
    async fn embed_batch_fails_after_retries() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("POST", "/api/embed")
            .with_status(500)
            .expect_at_least(3)
            .create_async()
            .await;
        let emb = OllamaEmbedder::new(server.url(), "bge-m3");
        let err = emb.embed_batch(&["a"]).await.unwrap_err();
        assert_eq!(err.code(), "embedding-failed");
    }

    #[tokio::test]
    async fn embed_empty_returns_empty() {
        let emb = OllamaEmbedder::new("http://unused", "bge-m3");
        let out = emb.embed_batch(&[]).await.unwrap();
        assert!(out.is_empty());
    }
}
