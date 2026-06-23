//! PP-OCR 模型路径解析 + 存在性检查（~/.local/share/attune-docs/models/ppocr/）。

use std::path::PathBuf;

/// 模型目录：$XDG_DATA_HOME 或 ~/.local/share 下的 attune-docs/models/ppocr。
pub fn models_dir() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("attune-docs/models/ppocr")
}

/// 4 个必需文件齐全才算就绪。
pub fn models_present() -> bool {
    let d = models_dir();
    ["ch_PP-OCRv5_det_mobile.onnx", "ch_ppocr_mobile_v2.0_cls.onnx", "ch_PP-OCRv5_rec_mobile.onnx", "ppocr_keys_v1.txt"]
        .iter()
        .all(|f| d.join(f).exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_dir_respects_xdg_data_home() {
        // SAFETY: 单线程测试内设置 env，仅本测试读取。
        std::env::set_var("XDG_DATA_HOME", "/tmp/attune-docs-test-xdg");
        let d = models_dir();
        assert!(d.ends_with("attune-docs/models/ppocr"));
        assert!(d.starts_with("/tmp/attune-docs-test-xdg"));
        std::env::remove_var("XDG_DATA_HOME");
    }

    #[test]
    fn models_present_false_when_dir_missing() {
        std::env::set_var("XDG_DATA_HOME", "/tmp/attune-docs-test-absent-xyz");
        assert!(!models_present());
        std::env::remove_var("XDG_DATA_HOME");
    }
}
