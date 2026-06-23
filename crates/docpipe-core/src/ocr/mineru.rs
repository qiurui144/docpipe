//! MinerUBackend — HTTP sidecar OCR（表格结构还原）。
//! 健康探测失败时由 facade 回退 KreuzbergBackend。

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;

use super::{OcrBackend, OcrResult};
use crate::error::{DocError, Result};
use crate::types::{BBox, TextBlock};

pub struct MinerUBackend {
    url: String,
    client: reqwest::Client,
    timeout_secs: u64,
}

#[derive(Deserialize)]
struct MinerUResp {
    blocks: Vec<MinerUBlock>,
}

#[derive(Deserialize)]
struct MinerUBlock {
    text: String,
    /// [x, y, w, h] 像素坐标
    bbox: [u32; 4],
    confidence: f32,
}

impl MinerUBackend {
    /// 构造函数。`url` 末尾斜线自动去掉；默认 30 s 请求超时。
    pub fn new(url: impl Into<String>) -> Self {
        let url = url.into().trim_end_matches('/').to_string();
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("reqwest client build failed");
        Self {
            url,
            client,
            timeout_secs: 30,
        }
    }

    /// 健康探测：GET {url}/health。5 s 探测超时；任何错误（含连接失败/超时）→ false。
    pub async fn health(&self) -> bool {
        let probe = self
            .client
            .get(format!("{}/health", self.url))
            .timeout(Duration::from_secs(5))
            .send()
            .await;
        matches!(probe, Ok(r) if r.status().is_success())
    }
}

#[async_trait]
impl OcrBackend for MinerUBackend {
    /// POST multipart `file` 字段到 `{url}/file_parse`。
    /// 响应 JSON 格式：`{"blocks":[{"text":"…","bbox":[x,y,w,h],"confidence":0.9}]}`。
    /// 超时 → `DocError::MineruTimeout`；非 2xx → `DocError::OcrBackendUnavailable`。
    async fn recognize(&self, page_image: &[u8], _dpi: u32) -> Result<OcrResult> {
        let part = reqwest::multipart::Part::bytes(page_image.to_vec())
            .file_name("page.png")
            .mime_str("image/png")
            .map_err(|e| DocError::Other(format!("mineru multipart mime: {e}")))?;
        let form = reqwest::multipart::Form::new().part("file", part);

        let resp = self
            .client
            .post(format!("{}/file_parse", self.url))
            .timeout(Duration::from_secs(self.timeout_secs))
            .multipart(form)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    DocError::MineruTimeout
                } else {
                    DocError::OcrBackendUnavailable(format!("mineru request error: {e}"))
                }
            })?;

        if !resp.status().is_success() {
            return Err(DocError::OcrBackendUnavailable(format!(
                "mineru returned HTTP {}",
                resp.status()
            )));
        }

        let parsed: MinerUResp = resp
            .json()
            .await
            .map_err(|e| DocError::Other(format!("mineru json parse: {e}")))?;

        let mut blocks = Vec::with_capacity(parsed.blocks.len());
        let mut conf_sum = 0.0f32;
        for b in &parsed.blocks {
            let [x, y, w, h] = b.bbox;
            blocks.push(TextBlock {
                text: b.text.clone(),
                bbox: BBox { x, y, w, h },
                confidence: b.confidence,
            });
            conf_sum += b.confidence;
        }
        let avg_confidence = if blocks.is_empty() {
            None
        } else {
            Some(conf_sum / blocks.len() as f32)
        };

        Ok(OcrResult {
            blocks,
            avg_confidence,
        })
    }

    fn name(&self) -> &str {
        "mineru"
    }

    fn requires_gpu(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn health_unreachable() {
        // 端口 1 不可达，health 应返回 false
        let backend = MinerUBackend::new("http://127.0.0.1:1");
        assert!(!backend.health().await);
    }

    #[tokio::test]
    async fn health_ok() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/health")
            .with_status(200)
            .create_async()
            .await;
        let backend = MinerUBackend::new(server.url());
        assert!(backend.health().await);
    }

    #[tokio::test]
    async fn recognize_ok() {
        let mut server = mockito::Server::new_async().await;
        let body = r#"{"blocks":[{"text":"表格单元","bbox":[10,20,30,40],"confidence":0.97}]}"#;
        let _m = server
            .mock("POST", "/file_parse")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;
        let backend = MinerUBackend::new(server.url());
        let result = backend.recognize(b"fake-png-bytes", 300).await.unwrap();
        assert_eq!(result.blocks.len(), 1);
        assert_eq!(result.blocks[0].text, "表格单元");
        assert_eq!(result.blocks[0].bbox.x, 10);
        assert_eq!(result.blocks[0].bbox.y, 20);
        assert_eq!(result.blocks[0].bbox.w, 30);
        assert_eq!(result.blocks[0].bbox.h, 40);
        assert!((result.blocks[0].confidence - 0.97).abs() < 1e-4);
        assert!(result.avg_confidence.is_some());
    }

    #[tokio::test]
    async fn recognize_timeout() {
        // 使用 tokio 监听一个端口但从不发送响应来模拟超时（reqwest 建连成功但 read 挂起）
        use tokio::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        // 接受连接但不发送任何响应 → 让 reqwest 在读取时超时
        tokio::spawn(async move {
            let (_socket, _addr) = listener.accept().await.unwrap();
            // 故意挂起：持有 socket 不写任何数据
            tokio::time::sleep(Duration::from_secs(10)).await;
        });

        let url = format!("http://127.0.0.1:{port}");
        let mut backend = MinerUBackend::new(url);
        backend.timeout_secs = 1; // 缩短超时便于测试

        let err = backend.recognize(b"fake", 300).await.unwrap_err();
        assert!(
            matches!(err, DocError::MineruTimeout),
            "expected MineruTimeout, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn recognize_non2xx() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("POST", "/file_parse")
            .with_status(503)
            .with_body("service unavailable")
            .create_async()
            .await;
        let backend = MinerUBackend::new(server.url());
        let err = backend.recognize(b"fake", 300).await.unwrap_err();
        assert!(
            matches!(err, DocError::OcrBackendUnavailable(_)),
            "expected OcrBackendUnavailable, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn name_is_mineru() {
        let backend = MinerUBackend::new("http://localhost:1234");
        assert_eq!(backend.name(), "mineru");
    }
}
