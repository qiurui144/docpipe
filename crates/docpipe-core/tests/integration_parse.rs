//! 真实 fixture 集成测试 — 需 ONNX 模型 + PDFium 库 + PII fixture（本地/RC 机器跑）。
//! CI 默认 skip（#[ignore]）；RC 验收用 `cargo test --include-ignored` 跑。

use std::sync::Arc;

use async_trait::async_trait;
use docpipe_core::ocr::{OcrBackend, OcrResult};
use docpipe_core::ocr::kreuzberg::KreuzbergBackend;
use docpipe_core::parser::pdf::PdfParser;
use docpipe_core::parser::DocParser;
use docpipe_core::types::ParseConfig;

fn fixture(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// OCR 伪后端：text-layer 路径不得调用 recognize，调用即 panic。
struct NoOcr;

#[async_trait]
impl OcrBackend for NoOcr {
    async fn recognize(
        &self,
        _page_image: &[u8],
        _dpi: u32,
    ) -> docpipe_core::error::Result<OcrResult> {
        panic!("text-layer 路径不应调用 OCR");
    }

    fn name(&self) -> &str {
        "no-ocr"
    }
}

/// 银行流水扫描件 OCR 路径：提取已知对方户名，置信度低块比例 < 20%。
/// 需要：ONNX 模型 + PDFium + tests/fixtures/bank_page1.pdf（PII，不入仓）。
#[tokio::test]
#[ignore = "requires ONNX models + PDFium + PII fixture"]
async fn test_bank_pdf_ocr_path() {
    let ocr = Arc::new(KreuzbergBackend::new().expect("PP-OCR ONNX models must be present"));
    let parser = PdfParser::new(ocr);
    let bytes = std::fs::read(fixture("bank_page1.pdf")).expect("bank_page1.pdf fixture must be present");
    let doc = parser
        .parse(
            &bytes,
            &ParseConfig { ocr: true, table_structure: false, max_pages: Some(1), dpi: 300 },
        )
        .await
        .unwrap();

    let expected: serde_json::Value = serde_json::from_slice(
        &std::fs::read(fixture("bank_page1_expected.json")).expect("golden file must be present"),
    )
    .unwrap();

    let page_text = &doc.pages[0].text;

    // 对方户名断言：golden JSON 中列出的每个名称必须出现在页面文本中。
    for name in expected["names"].as_array().unwrap() {
        let n = name.as_str().unwrap();
        assert!(page_text.contains(n), "缺少对方户名: {n}");
    }

    // 置信度门槛（spec §9 happy path）：低置信度非空块占比 < 20%。
    let low_conf: Vec<_> = doc.pages[0]
        .blocks
        .iter()
        .filter(|b| b.confidence < 0.95 && !b.text.trim().is_empty())
        .collect();
    let total = doc.pages[0].blocks.len().max(1);
    let ratio = low_conf.len() as f32 / total as f32;
    assert!(ratio < 0.2, "低置信度块比例过高: {ratio:.2} (low={} total={total})", low_conf.len());
}

/// 字层 PDF 路径：text-layer 检测到时绝不调用 OCR backend，且 ocr_used == false。
/// golden 片段必须出现在合并文本中。
/// 需要：PDFium + tests/fixtures/text_layer_sample.pdf（无 PII，但运行时需 PDFium 库）。
#[tokio::test]
#[ignore = "requires PDFium + text-layer fixture"]
async fn test_text_layer_no_ocr() {
    let parser = PdfParser::new(Arc::new(NoOcr));
    let bytes = std::fs::read(fixture("text_layer_sample.pdf"))
        .expect("text_layer_sample.pdf fixture must be present");
    let doc = parser
        .parse(&bytes, &ParseConfig::default())
        .await
        .unwrap();

    // 字层路径断言：ocr_used 必须为 false（NoOcr 若被调用则 panic）。
    assert!(!doc.ocr_used, "text-layer PDF 不应设置 ocr_used=true");

    let combined: String = doc.pages.iter().map(|p| p.text.as_str()).collect::<Vec<_>>().join("\n");
    let expected = std::fs::read_to_string(fixture("zuhe_page2_expected.txt"))
        .expect("golden file zuhe_page2_expected.txt must be present");

    // golden 片段断言：stable phrase 必须出现在文档中。
    assert!(
        combined.contains(expected.trim()),
        "字层文本缺失 golden 片段: {:?}",
        expected.trim()
    );
}
