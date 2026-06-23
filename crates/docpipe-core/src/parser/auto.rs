//! 格式自动检测 — magic bytes 优先，ZIP 容器内再嗅探 DOCX vs EPUB（spec §3 format auto-detect）。

use crate::error::{DocError, Result};
use crate::types::DocFormat;

/// 根据文件头部字节判定格式。ZIP 容器（DOCX/EPUB）需读内部 marker 区分。
pub fn detect_format(bytes: &[u8]) -> Result<DocFormat> {
    if bytes.starts_with(b"%PDF") {
        return Ok(DocFormat::Pdf);
    }
    // ZIP 容器：PK\x03\x04。DOCX 含 word/，EPUB 含 epub+zip mimetype。
    if bytes.starts_with(&[0x50, 0x4B, 0x03, 0x04]) {
        // 仅扫描前 4KB 足以覆盖 ZIP local file header 区的 marker。
        let window = &bytes[..bytes.len().min(4096)];
        if contains_subslice(window, b"epub+zip") {
            return Ok(DocFormat::Epub);
        }
        if contains_subslice(window, b"word/") {
            return Ok(DocFormat::Docx);
        }
        return Err(DocError::FormatUnsupported);
    }
    // HTML：宽松检测开头（跳过 BOM/空白）含 <!doctype html 或 <html。
    let head: String = bytes
        .iter()
        .take(512)
        .map(|&b| b as char)
        .collect::<String>()
        .to_ascii_lowercase();
    let trimmed = head.trim_start();
    if trimmed.starts_with("<!doctype html") || trimmed.starts_with("<html") {
        return Ok(DocFormat::Html);
    }
    Err(DocError::FormatUnsupported)
}

fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::detect_format;
    use crate::types::DocFormat;

    #[test]
    fn detects_pdf_by_magic() {
        let bytes = b"%PDF-1.7\n....";
        assert_eq!(detect_format(bytes).unwrap(), DocFormat::Pdf);
    }

    #[test]
    fn detects_docx_zip_magic() {
        // DOCX/EPUB are ZIP containers: PK\x03\x04. DOCX has [Content_Types].xml; here we
        // assert the ZIP path resolves to Docx when the word/ marker is present.
        let mut bytes = vec![0x50, 0x4B, 0x03, 0x04];
        bytes.extend_from_slice(b"....word/document.xml....");
        assert_eq!(detect_format(&bytes).unwrap(), DocFormat::Docx);
    }

    #[test]
    fn detects_epub_zip_magic() {
        let mut bytes = vec![0x50, 0x4B, 0x03, 0x04];
        bytes.extend_from_slice(b"....mimetypeapplication/epub+zip....");
        assert_eq!(detect_format(&bytes).unwrap(), DocFormat::Epub);
    }

    #[test]
    fn detects_html_by_tag() {
        let bytes = b"<!DOCTYPE html><html><body>hi</body></html>";
        assert_eq!(detect_format(bytes).unwrap(), DocFormat::Html);
    }

    #[test]
    fn rejects_unknown_zip_pretending_pdf() {
        // bare ZIP with neither word/ nor epub marker → unsupported (adversarial: ZIP-as-PDF)
        let bytes = vec![0x50, 0x4B, 0x03, 0x04, 0x00, 0x00];
        assert!(detect_format(&bytes).is_err());
    }

    #[test]
    fn rejects_garbage() {
        let bytes = b"\x00\x01\x02not a document";
        assert!(detect_format(bytes).is_err());
    }
}
