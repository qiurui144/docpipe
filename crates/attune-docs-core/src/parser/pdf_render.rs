//! PDF 渲染 + 字层检测。pdf-extract 抽字层；pdfium-render 渲染页为图（自动应用旋转，spec §7）。

use crate::error::{DocError, Result};

#[derive(Debug, Clone)]
pub struct PageText {
    pub page_num: u32,
    pub text: String,
}

/// 字层判定：平均每页字符数 > 20 视为有字层（spec §3 OCR 决策树）。
pub fn is_text_layer(pages: &[PageText]) -> bool {
    if pages.is_empty() {
        return false;
    }
    let total: usize = pages.iter().map(|p| p.text.chars().count()).sum();
    (total as f64 / pages.len() as f64) > 20.0
}

/// 逐页抽字层。pdf-extract 按换页符 \u{0C} 近似分页；若无法分页则单页返回全文。
pub fn extract_text_layer(pdf_bytes: &[u8]) -> Result<Vec<PageText>> {
    let full = pdf_extract::extract_text_from_mem(pdf_bytes)
        .map_err(|e| DocError::Other(format!("pdf-extract: {e}")))?;
    let parts: Vec<&str> = full.split('\u{0C}').collect();
    let pages = parts
        .iter()
        .enumerate()
        .map(|(i, t)| PageText { page_num: (i + 1) as u32, text: t.to_string() })
        .collect();
    Ok(pages)
}

/// 总页数（pdfium 动态绑定）。
pub fn page_count(pdf_bytes: &[u8]) -> Result<u32> {
    let pdfium = pdfium()?
        .lock()
        .map_err(|_| DocError::Other("pdfium lock poisoned".into()))?;
    let doc = pdfium
        .load_pdf_from_byte_slice(pdf_bytes, None)
        .map_err(|e| DocError::Other(format!("pdfium load: {e}")))?;
    Ok(doc.pages().len() as u32)
}

/// 渲染指定页为 PNG bytes。dpi 决定缩放（PDF 基准 72 点/英寸）。旋转由 pdfium 自动应用。
pub fn render_page_png(pdf_bytes: &[u8], page_index: u32, dpi: u32) -> Result<Vec<u8>> {
    use pdfium_render::prelude::*;
    let pdfium = pdfium()?
        .lock()
        .map_err(|_| DocError::Other("pdfium lock poisoned".into()))?;
    let doc = pdfium
        .load_pdf_from_byte_slice(pdf_bytes, None)
        .map_err(|e| DocError::Other(format!("pdfium load: {e}")))?;
    let page = doc
        .pages()
        .get(page_index as PdfPageIndex)
        .map_err(|e| DocError::Other(format!("pdfium page {page_index}: {e}")))?;
    let scale = dpi as f32 / 72.0;
    let cfg = PdfRenderConfig::new().scale_page_by_factor(scale);
    let bitmap = page
        .render_with_config(&cfg)
        .map_err(|e| DocError::Other(format!("pdfium render: {e}")))?;
    let img = bitmap
        .as_image()
        .map_err(|e| DocError::Other(format!("pdfium bitmap: {e}")))?;
    let mut png: Vec<u8> = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|e| DocError::Other(format!("png encode: {e}")))?;
    Ok(png)
}

/// 进程级单次 PDFium 绑定。PDFium C 库是**全局单例**——`FPDF_InitLibrary` 每进程只能调用一次，
/// 重复 `bind_*` 会报 `PdfiumLibraryBindingsAlreadyInitialized`。因此全进程只绑定一次并缓存；
/// 又因 PDFium 非线程安全，用 `Mutex` 串行化所有 PDF 操作（page_count / render 共享同一实例）。
/// 绑定失败时把错误字符串缓存，后续每次调用复用同一错误（避免反复重试 init）。
fn pdfium() -> Result<&'static std::sync::Mutex<pdfium_render::prelude::Pdfium>> {
    use std::sync::{Mutex, OnceLock};
    static CELL: OnceLock<std::result::Result<Mutex<pdfium_render::prelude::Pdfium>, String>> =
        OnceLock::new();
    match CELL.get_or_init(|| bind_pdfium_once().map(Mutex::new).map_err(|e| e.to_string())) {
        Ok(m) => Ok(m),
        Err(s) => Err(DocError::Other(s.clone())),
    }
}

/// 绑定 PDFium 动态库（仅由 `pdfium()` 调用一次）。先找系统库；也接受 PDFIUM_DYNAMIC_LIB_PATH。
/// 该环境变量既可是**库文件完整路径**（如 `/usr/local/lib/libpdfium.so`，文档/Dockerfile 约定），
/// 也可是**含库的目录**——指向已存在的文件时直接 `bind_to_library`，否则按目录用
/// `pdfium_platform_library_name_at_path` 自动补全库文件名。两种写法都支持，消除歧义。
fn bind_pdfium_once() -> Result<pdfium_render::prelude::Pdfium> {
    use pdfium_render::prelude::*;
    if let Some(path) = std::env::var_os("PDFIUM_DYNAMIC_LIB_PATH") {
        let path_str: String = path.to_string_lossy().into_owned();
        // 指向已存在的文件 → 直接当库文件路径；否则当目录补全库文件名。两分支都调 bind_to_library。
        let bindings = if std::path::Path::new(&path_str).is_file() {
            Pdfium::bind_to_library(&path_str)
        } else {
            Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path(path_str.as_str()))
        }
        .map_err(|e| DocError::Other(format!("pdfium bind ({path_str}): {e}")))?;
        Ok(Pdfium::new(bindings))
    } else {
        let bindings = Pdfium::bind_to_system_library().map_err(|e| {
            DocError::Other(format!(
                "pdfium bind: {e}; install libpdfium or set PDFIUM_DYNAMIC_LIB_PATH"
            ))
        })?;
        Ok(Pdfium::new(bindings))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_text_layer_true_above_threshold() {
        let pages = vec![
            PageText { page_num: 1, text: "a".repeat(500) },
            PageText { page_num: 2, text: "b".repeat(500) },
        ];
        assert!(is_text_layer(&pages));
    }

    #[test]
    fn is_text_layer_false_for_scanned() {
        // 扫描件：pdf-extract 抽不到字层，平均 < 20 字符/页。
        let pages = vec![
            PageText { page_num: 1, text: "".into() },
            PageText { page_num: 2, text: "x".into() },
        ];
        assert!(!is_text_layer(&pages));
    }

    #[test]
    fn is_text_layer_false_for_empty() {
        assert!(!is_text_layer(&[]));
    }

    // 真实 fixture 测试：需要 PDFium 库 + 测试 PDF，CI 用 --include-ignored 跑。
    #[test]
    #[ignore = "requires PDFium lib + fixture PDF"]
    fn extract_text_layer_real_pdf() {
        let bytes = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/text_layer_sample.pdf"
        ))
        .unwrap();
        let pages = extract_text_layer(&bytes).unwrap();
        assert!(!pages.is_empty());
        assert!(is_text_layer(&pages));
    }
}
