//! 共享应用状态 — 持有装配好的 Docpipe + tier 信息。

use std::sync::Arc;

use docpipe_core::store::SqliteVecStore;
use docpipe_core::{Docpipe, DocpipeBuilder};

use crate::config::Config;

pub struct AppState {
    pub sdk: Docpipe,
    pub ram_tier: String,
    pub mineru_configured: bool,
}

impl AppState {
    pub fn from_config(cfg: &Config) -> Result<Self, String> {
        use docpipe_core::embedder::OllamaEmbedder;
        use docpipe_core::ocr::kreuzberg::KreuzbergBackend;
        use docpipe_core::ocr::mineru::MinerUBackend;

        let ocr = KreuzbergBackend::new().map_err(|e| format!("kreuzberg init: {e}"))?;
        let store =
            SqliteVecStore::new(&cfg.sqlite_path).map_err(|e| format!("sqlite init: {e}"))?;
        let embedder = OllamaEmbedder::new(&cfg.ollama_url, &cfg.embed_model);
        let mut builder = DocpipeBuilder::new()
            .ocr_backend(Arc::new(ocr))
            .vector_store(Arc::new(store))
            .embedder(Arc::new(embedder));
        let mineru_configured = cfg.mineru_url.is_some();
        if let Some(url) = &cfg.mineru_url {
            let mineru = MinerUBackend::new(url.clone());
            builder = builder.mineru(Arc::new(mineru));
        }
        let sdk = builder.build().map_err(|e| format!("build sdk: {e}"))?;
        let ram_tier = if mineru_configured {
            "full".to_string()
        } else {
            "lite".to_string()
        };
        Ok(Self {
            sdk,
            ram_tier,
            mineru_configured,
        })
    }

    /// 测试用：不接真实 OCR/Ollama，用内存 store + dummy backends。
    #[cfg(test)]
    pub fn for_test() -> Self {
        use async_trait::async_trait;
        use docpipe_core::embedder::Embedder;
        use docpipe_core::ocr::{OcrBackend, OcrResult};

        struct NoOcr;
        #[async_trait]
        impl OcrBackend for NoOcr {
            async fn recognize(
                &self,
                _i: &[u8],
                _d: u32,
            ) -> docpipe_core::error::Result<OcrResult> {
                Ok(OcrResult {
                    blocks: vec![],
                    avg_confidence: None,
                })
            }
            fn name(&self) -> &str {
                "no-ocr"
            }
        }

        struct NoEmbed;
        #[async_trait]
        impl Embedder for NoEmbed {
            async fn embed_batch(&self, t: &[&str]) -> docpipe_core::error::Result<Vec<Vec<f32>>> {
                Ok(t.iter()
                    .map(|s| vec![s.chars().count() as f32, 0.0, 0.0])
                    .collect())
            }
            fn dim(&self) -> usize {
                3
            }
            fn model_name(&self) -> &str {
                "no-embed"
            }
        }

        let sdk = DocpipeBuilder::new()
            .ocr_backend(Arc::new(NoOcr))
            .vector_store(Arc::new(SqliteVecStore::in_memory().unwrap()))
            .embedder(Arc::new(NoEmbed))
            .build()
            .unwrap();
        Self {
            sdk,
            ram_tier: "lite".into(),
            mineru_configured: false,
        }
    }
}
