//! KreuzbergBackend — PP-OCRv4 ONNX 原生推理（kreuzberg-paddle-ocr OcrLite）。
//!
//! OcrLite 内部不是 Sync，必须 Mutex 包裹。detect_from_path 吃文件路径，故 page_image bytes
//! 先落 tempfile 再喂入。参数遵循 RapidOCR 官方默认值（RapidOCR 官方默认值）。

use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

use async_trait::async_trait;
use kreuzberg_paddle_ocr::OcrLite;

use super::models::models_dir;
use super::{OcrBackend, OcrResult};
use crate::error::{DocError, Result};
use crate::types::{BBox, TextBlock};

/// KreuzbergBackend — 持有已初始化的 OcrLite session（Mutex 保证线程安全）。
// OcrLite 未实现 Debug，手动实现以满足 Result::unwrap_err 等工具链要求。
pub struct KreuzbergBackend {
    /// OcrLite 内部不是 Sync — 用 Mutex 保证 trait Send+Sync。
    /// OCR 是低频 + 单次推理 ~500ms-3s，单线程串行不是瓶颈。
    inner: Mutex<OcrLite>,
}

impl KreuzbergBackend {
    /// 从默认模型目录构造。模型缺失 → Err(OcrBackendUnavailable)。
    pub fn new() -> Result<Self> {
        Self::from_models_dir(&models_dir())
    }

    /// 从指定目录构造。便于测试注入不同模型路径。
    pub fn from_models_dir(dir: &Path) -> Result<Self> {
        let det = dir.join("ch_PP-OCRv5_det_mobile.onnx");
        let cls = dir.join("ch_ppocr_mobile_v2.0_cls.onnx");
        let rec = dir.join("ch_PP-OCRv5_rec_mobile.onnx");
        let dict = dir.join("ppocr_keys_v1.txt");

        for p in [&det, &cls, &rec, &dict] {
            if !p.exists() {
                return Err(DocError::OcrBackendUnavailable(format!(
                    "missing model file: {}",
                    p.display()
                )));
            }
        }

        let n_threads = std::thread::available_parallelism()
            .map(|n| n.get().min(8))
            .unwrap_or(4);

        let mut ocr = OcrLite::new();
        ocr.init_models_with_dict(
            det.to_str()
                .ok_or_else(|| DocError::Other("non-UTF8 det path".into()))?,
            cls.to_str()
                .ok_or_else(|| DocError::Other("non-UTF8 cls path".into()))?,
            rec.to_str()
                .ok_or_else(|| DocError::Other("non-UTF8 rec path".into()))?,
            dict.to_str()
                .ok_or_else(|| DocError::Other("non-UTF8 dict path".into()))?,
            n_threads,
        )
        .map_err(|e| DocError::OcrBackendUnavailable(format!("init_models_with_dict: {e}")))?;

        Ok(Self {
            inner: Mutex::new(ocr),
        })
    }
}

impl std::fmt::Debug for KreuzbergBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KreuzbergBackend").finish_non_exhaustive()
    }
}

/// `Default` 实现：模型缺失时 panic（用于 builder 便利路径，失败路径应使用 `new()`）。
impl Default for KreuzbergBackend {
    fn default() -> Self {
        Self::new().unwrap_or_else(|e| {
            panic!(
                "KreuzbergBackend::default() requires PP-OCR models at {:?}; \
                 download them first or use KreuzbergBackend::new() for fallible construction. \
                 Error: {e}",
                models_dir()
            )
        })
    }
}

#[async_trait]
impl OcrBackend for KreuzbergBackend {
    async fn recognize(&self, page_image: &[u8], _dpi: u32) -> Result<OcrResult> {
        // detect_from_path 要文件路径 → 先把图像字节落临时文件（.png 后缀）
        let mut tmp = tempfile::Builder::new()
            .suffix(".png")
            .tempfile()
            .map_err(DocError::Io)?;
        tmp.write_all(page_image).map_err(DocError::Io)?;
        tmp.flush().map_err(DocError::Io)?;

        let path_str = tmp
            .path()
            .to_str()
            .ok_or_else(|| DocError::Other("non-UTF8 temp path".into()))?;

        // 锁 OcrLite（非 Sync，单次持锁推理）
        let lock = self
            .inner
            .lock()
            .map_err(|_| DocError::Other("OcrLite lock poisoned".into()))?;

        // 推理参数（遵循 RapidOCR 官方默认值）：
        //   padding=50         短边 border padding
        //   max_side=2048      长边最大分辨率
        //   box_score=0.6      detection 置信度阈值
        //   box_thresh=0.3     DBNet binarization 阈值
        //   unclip=1.6         文本框扩张比
        //   do_angle=true      做方向分类
        //   most_angle=true    全图统一方向
        let result = lock
            .detect_from_path(path_str, 50, 2048, 0.6, 0.3, 1.6, true, true)
            .map_err(|e| DocError::OcrBackendUnavailable(format!("detect_from_path: {e}")))?;

        // lock 先 drop，再 drop tmp（lock 与 tmp 释放顺序）
        drop(lock);

        // 转换 kreuzberg TextBlock → docpipe TextBlock
        let mut blocks = Vec::with_capacity(result.text_blocks.len());
        let mut conf_sum = 0.0f32;
        let mut conf_n = 0u32;

        for b in &result.text_blocks {
            if b.text.is_empty() {
                continue;
            }
            // box_points: [tl, tr, br, bl]，Point { x: u32, y: u32 }
            let xs: Vec<u32> = b.box_points.iter().map(|p| p.x).collect();
            let ys: Vec<u32> = b.box_points.iter().map(|p| p.y).collect();
            let x = *xs.iter().min().unwrap_or(&0);
            let y = *ys.iter().min().unwrap_or(&0);
            let w = xs.iter().max().unwrap_or(&0).saturating_sub(x);
            let h = ys.iter().max().unwrap_or(&0).saturating_sub(y);

            blocks.push(TextBlock {
                text: b.text.clone(),
                bbox: BBox { x, y, w, h },
                confidence: b.text_score,
            });
            conf_sum += b.text_score;
            conf_n += 1;
        }

        let avg_confidence = if conf_n > 0 {
            Some(conf_sum / conf_n as f32)
        } else {
            None
        };

        Ok(OcrResult {
            blocks,
            avg_confidence,
        })
    }

    fn name(&self) -> &str {
        "kreuzberg"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_errs_when_models_missing() {
        // 指向必然不存在的临时目录 → 应返回 Err(OcrBackendUnavailable)
        let absent = std::path::Path::new("/tmp/docpipe-kreuzberg-absent-xyz");
        let result = KreuzbergBackend::from_models_dir(absent);
        assert!(
            result.is_err(),
            "from_models_dir should fail when model files are absent"
        );
        let err = result.unwrap_err();
        assert!(
            matches!(err, DocError::OcrBackendUnavailable(_)),
            "expected OcrBackendUnavailable, got: {err:?}"
        );
    }

    #[test]
    fn backend_is_send_and_sync() {
        // 编译期断言：KreuzbergBackend 必须 Send + Sync（OcrBackend trait 要求）
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<KreuzbergBackend>();
    }
}
