# docpipe

> [English](./README.md)

**可自托管的文档处理管线 —— 解析 · OCR · 分块 · 向量化 · 检索 · 批注。**

一个通用、行业无关的文档处理 SDK。纯 Rust 核心 + 可插拔 trait 体系，外包一层 HTTP 服务，并提供 Python 与
TypeScript 客户端。把 PDF / DOCX / HTML（**文字层或扫描件**）转成结构化、可检索、可批注的内容 —— 全程跑在你
自己的基础设施上，无需任何云端 API。

> 状态：**v0.1.0** —— 核心管线已实现，并在 Linux x64 与 Windows x64 上完成端到端验证。
> 见[已知限制](#已知限制)。

## 为什么

每个要处理文档的产品都在重复造同一条管线（解析 → OCR → 分块 → 向量化 → 存储 → 批注），各自质量参差不齐。
`docpipe` 把这条管线抽取为一个独立的库 + HTTP 服务，让任意技术栈（Rust / Python / TypeScript / 任何能讲 HTTP
的语言）都能直接获得同样的能力，而不必重复发明。

## 特性

- **解析** —— PDF（自动检测文字层，无字层回退 OCR）、DOCX、HTML，带格式自动识别。
- **OCR** —— PP-OCRv4 ONNX（经 `kreuzberg-paddle-ocr`），Rust 原生、无需 Python 运行时。能读扫描件与带防伪水印
  的页面（如银行流水）—— 这些场景 Tesseract 会失败。
- **分层后端** —— *Lite*（SQLite + 进程内 OCR，无额外容器）与 *Full*（加 MinerU sidecar 做表格结构还原，带健康
  探针自动回退到内置 OCR）。
- **分块** —— 语义感知、尊重句边界的滑动窗口，重叠比例可配。
- **向量化** —— 任意 OpenAI / Ollama 兼容的 `/api/embed` 端点。
- **向量存储** —— SQLite + `sqlite-vec`（Lite）；Weaviate 规划中（v1.1）。
- **批注** —— 行业无关的 `AnnotatableItem` + `TextLocator`，带内容哈希以检测文档漂移；AI 批注与人工批注共用一套
  数据模型。
- **可插拔** —— `DocParser`、`OcrBackend`、`Embedder`、`VectorStore` 都是 trait，可自带实现。

## 架构

```
            HTTP /v1/*                        Rust crate（可直接链接）
  Python / TS / 任意客户端  ─┐        ┌─  docpipe-core（纯库，零 HTTP 依赖）
                             ▼        ▼
                      docpipe-server (axum)  ──►  parser · ocr · chunker
                                                   embedder · store · annotator
                                                        │
                                       KreuzbergBackend（PP-OCRv4 ONNX，默认）
                                       MinerUBackend   （HTTP sidecar，可选）
```

| 组件 | crate / 包 | 职责 |
|---|---|---|
| `docpipe-core` | crates/docpipe-core | 纯 Rust 库：traits、types、parser、OCR、chunker、embedder、store、annotator |
| `docpipe-server` | crates/docpipe-server | 暴露 `/v1/*` 的 axum HTTP 服务 |
| Python 客户端 | `docpipe-client`（PyPI） | `from docpipe import DocpipeClient` |
| TypeScript 客户端 | `@qiurui144/docpipe`（npm） | `import { DocpipeClient } from "@qiurui144/docpipe"` |

## 快速开始

### 启动服务

```bash
# 运行时依赖：一个 PDFium 动态库 + PP-OCR ONNX 模型 —— 见 DEVELOP.md
export PDFIUM_DYNAMIC_LIB_PATH=/path/to/libpdfium.so   # 库文件路径或所在目录均可
export OLLAMA_URL=http://localhost:11434
export EMBED_MODEL=bge-m3
cargo run -p docpipe-server                            # 监听 0.0.0.0:8200
```

或用 Docker：

```bash
docker compose -f docker/lite/docker-compose.yml up    # Lite 层（SQLite，无 MinerU）
docker compose -f docker/full/docker-compose.yml up    # Full 层（+ MinerU sidecar）
```

### HTTP API

| 方法 | 路径 | 职责 |
|---|---|---|
| POST | `/v1/parse` | multipart 文件 → `ParsedDocument`（text + blocks + tables） |
| POST | `/v1/chunk` | 文本 → 语义分块 |
| POST | `/v1/embed` | 文本 → 向量 |
| POST | `/v1/search` | query → 最近邻分块 |
| POST | `/v1/annotate` | 创建一个批注项 |
| GET  | `/v1/health` | 后端就绪状态 + 部署层级 |

```bash
curl -F file=@scan.pdf http://localhost:8200/v1/parse
```

完整规范见 [`openapi.yaml`](./openapi.yaml)。

### Rust（直接链接核心库）

```rust
use docpipe_core::{DocpipeBuilder, ParseConfig};

let sdk = DocpipeBuilder::new()
    .ocr_backend(std::sync::Arc::new(KreuzbergBackend::new()?))
    .vector_store(std::sync::Arc::new(SqliteVecStore::new("docs.db")?))
    .embedder(std::sync::Arc::new(OllamaEmbedder::new("http://localhost:11434", "bge-m3")))
    .build()?;

let parsed = sdk.parse(&bytes, ParseConfig::default()).await?;
let ids    = sdk.ingest(&parsed, "default").await?;
let hits   = sdk.search("梁素燕 2019 跨行汇款", "default", 5).await?;
```

### Python

```python
from docpipe import DocpipeClient
doc = DocpipeClient("http://localhost:8200").parse("scan.pdf")
```

## 已验证

- `cargo test --workspace`：54 个用例通过（Linux x64 **与** Windows x64 / MSVC），clippy 无告警。
- 在一台 Windows Intel 真机上完成端到端验证：完整 MSVC 构建（ONNX + sqlite-vec + PDFium 全部链接成功）、服务
  起服，并通过 `/v1/parse` 把一份扫描中文 PDF 正确 OCR（卡号、金额、日期全部抽取无误），向量化走真实 Ollama。

## 已知限制

- **EPUB** 解析与 **Weaviate** 向量后端规划在 v1.1（当前 EPUB 返回 `format-unsupported`）。
- **v1.0 模型需手动准备** —— PP-OCR ONNX 模型 + 字典须放在 `~/.local/share/docpipe/models/ppocr/`；缺失时服务
  启动即失败。自动下载在 v1.1。见 [DEVELOP.md](./DEVELOP.md)（注意字典必须**无 BOM、LF 换行**）。
- **`sqlite-vec` 已 vendored**（打过补丁），位于 `vendor/`，用于绕过上游 `0.1.10-alpha.4` crates.io 包缺文件的
  问题 —— 见 `vendor/sqlite-vec/PATCH-NOTES.md`。
- 检索 `score` 是 `1 − distance`（基于 `sqlite-vec` 的 L2 度量，单调最近优先；不是归一化的 cosine 相似度）。

## 开发

工作区布局、运行时依赖（PDFium、ONNX 模型）、环境变量表、构建/测试、客户端维护，详见 [DEVELOP.md](./DEVELOP.md)。

## 许可证

Apache-2.0 —— 见 [LICENSE](./LICENSE)。Vendored 的 `sqlite-vec` 为 MIT/Apache-2.0（见 `vendor/sqlite-vec/`）。
