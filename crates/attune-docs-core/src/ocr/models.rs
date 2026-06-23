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

    // 序列化锁：两个测试都会修改进程级 XDG_DATA_HOME，并行运行会产生竞争；
    // 持有此锁期间独占 env 修改权，彻底消除竞态。
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn models_dir_respects_xdg_data_home() {
        let _guard = ENV_LOCK.lock().unwrap();
        // 保存原值，测试后恢复，避免污染其他测试。
        let prev = std::env::var_os("XDG_DATA_HOME");
        std::env::set_var("XDG_DATA_HOME", "/tmp/attune-docs-test-xdg");
        let d = models_dir();
        // 先恢复再断言，确保 panic 不会跳过恢复逻辑。
        match prev {
            Some(v) => std::env::set_var("XDG_DATA_HOME", v),
            None => std::env::remove_var("XDG_DATA_HOME"),
        }
        assert!(d.ends_with("attune-docs/models/ppocr"));
        assert!(d.starts_with("/tmp/attune-docs-test-xdg"));
    }

    #[test]
    fn models_present_false_when_dir_missing() {
        let _guard = ENV_LOCK.lock().unwrap();
        // 保存原值，测试后恢复。
        let prev = std::env::var_os("XDG_DATA_HOME");
        std::env::set_var("XDG_DATA_HOME", "/tmp/attune-docs-test-absent-xyz");
        let result = models_present();
        match prev {
            Some(v) => std::env::set_var("XDG_DATA_HOME", v),
            None => std::env::remove_var("XDG_DATA_HOME"),
        }
        assert!(!result);
    }
}
