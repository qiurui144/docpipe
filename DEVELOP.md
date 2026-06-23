# Attune-Docs SDK — 开发者手册

## 目录

- [工作区布局](#工作区布局)
- [构建](#构建)
- [运行时依赖](#运行时依赖)
  - [PDFium](#pdfium)
  - [ONNX 模型自动下载](#onnx-模型自动下载)
- [服务器运行](#服务器运行)
  - [环境变量说明](#环境变量说明)
- [运行测试](#运行测试)
- [Lite vs Full 部署决策](#lite-vs-full-部署决策)
- [客户端生成说明](#客户端生成说明)
- [从 attune-enterprise docling-serve 迁移](#从-attune-enterprise-docling-serve-迁移)
- [已知限制与路线图](#已知限制与路线图)

---

## 工作区布局

```
attune-docs/
├── Cargo.toml              # workspace root，成员：attune-docs-core, attune-docs-server
├── Cargo.lock
├── crates/
│   ├── attune-docs-core/   # 库 crate：traits、types、parser、OCR、chunker、embedder、store
│   └── attune-docs-server/ # binary crate：axum HTTP 服务
├── clients/
│   ├── python/             # Python 客户端（pytest 测试）
│   └── typescript/         # TypeScript 客户端（vitest 测试）
├── docker/
│   ├── lite/               # SQLite 单容器 compose
│   ├── full/               # + MinerU sidecar compose
│   └── pdfium/             # gitignored：libpdfium.so 预构建二进制
├── vendor/
│   └── sqlite-vec/         # patched sqlite-vec（解决 alpha.4 diskann 文件缺失）
├── Dockerfile
└── openapi.yaml
```

---

## 构建

### 开发构建

```bash
# 安装 Rust 工具链（若使用 rustup）
rustup toolchain install 1.85

# 构建所有 crate
cargo build --workspace

# 仅构建 server binary
cargo build -p attune-docs-server
```

### Release 构建

```bash
cargo build --release --bin attune-docs-server
# 产物：target/release/attune-docs-server
```

### Docker 镜像构建

构建前需准备 `docker/pdfium/libpdfium.so`（见下方 [PDFium](#pdfium) 章节）。

```bash
# Lite 部署
docker compose -f docker/lite/docker-compose.yml build

# Full 部署
docker compose -f docker/full/docker-compose.yml build
```

---

## 运行时依赖

### PDFium

`pdfium-render 0.9` 需要动态链接 `libpdfium.so`。预构建二进制来自：

```
https://github.com/bblanchon/pdfium-binaries/releases
```

**匹配版本**：下载时选择与 `pdfium-render = "0.9"` ABI 匹配的版本。查阅
`crates/attune-docs-core/Cargo.toml` 中 `pdfium-render` 的确切版本号，然后在
pdfium-binaries release 页面选取对应的构建（通常是最近稳定 chromium pdfium tag）。

**本地开发安装**：

```bash
# 下载并放置（示例，具体 URL 参照 pdfium-binaries release 页面）
mkdir -p docker/pdfium
curl -L https://github.com/bblanchon/pdfium-binaries/releases/download/chromium%2F6721/pdfium-linux-x64.tgz \
  | tar xz -C /tmp
cp /tmp/lib/libpdfium.so docker/pdfium/libpdfium.so

# 配置本地运行时环境变量
export PDFIUM_DYNAMIC_LIB_PATH="$(pwd)/docker/pdfium/libpdfium.so"
export LD_LIBRARY_PATH="$(pwd)/docker/pdfium:$LD_LIBRARY_PATH"
```

**CI 缓存**：在 GitHub Actions 中将 `docker/pdfium/` 加入 cache key（按 pdfium 版本 tag 区分），避免重复下载。

> `docker/pdfium/` 已加入 `.gitignore`，不入版本库。

### PP-OCR ONNX 模型（v1.0 需手动准备）

v1.0 **不自动下载模型**：`KreuzbergBackend::new()` 在模型缺失时返回 `ocr-backend-unavailable`，
服务启动即失败（`AppState::from_config` 会因此退出）。必须先把以下 4 个文件放到模型目录：

```
~/.local/share/attune-docs/models/ppocr/
  ├── ch_PP-OCRv5_det_mobile.onnx        # ~4.7 MB  检测  (源 SWHL/RapidOCR PP-OCRv4 det)
  ├── ch_ppocr_mobile_v2.0_cls.onnx      # ~0.6 MB  方向分类
  ├── ch_PP-OCRv5_rec_mobile.onnx        # ~10.9 MB 识别  (源 SWHL/RapidOCR PP-OCRv4 rec)
  └── ppocr_keys_v1.txt                  # 字典 (PaddleOCR ppocr_keys_v1.txt 包装而成)
```

目录可经 `XDG_DATA_HOME` 覆盖；容器内对应 `/root/.local/share/attune-docs/models/`（compose 已挂载 volume）。

**⚠️ 字典格式硬约束（否则 OCR 输出乱码且置信度仍很高）**：`ppocr_keys_v1.txt` 必须是
**无 BOM、LF 换行的 UTF-8**，内容为 `#\n` + 原始字典（每行一字）+ `\n ` 结尾（`#` 前缀行 + 末尾空格行
是 kreuzberg CTC blank 约定）。用 PowerShell `Set-Content -Encoding utf8`（会写入 BOM）准备字典会
导致整张索引→字符映射错位 —— 请用 `python` / `printf` 等不写 BOM 的方式生成。

> v1.1 计划加入模型自动下载（带 S8 多源 failover），届时本节改为「首次启动自动下载」。

---

## 服务器运行

### 本地快速启动

```bash
export PDFIUM_DYNAMIC_LIB_PATH="$(pwd)/docker/pdfium/libpdfium.so"
export LD_LIBRARY_PATH="$(pwd)/docker/pdfium:$LD_LIBRARY_PATH"
export DATABASE_URL="sqlite:///tmp/attune-docs-dev.db"
export OLLAMA_URL="http://localhost:11434"
export EMBED_MODEL="bge-m3"

cargo run -p attune-docs-server
# 默认监听 0.0.0.0:8200
```

### 环境变量说明

| 环境变量 | 默认值 | 说明 |
|---------|--------|------|
| `BIND_ADDR` | `0.0.0.0:8200` | HTTP 监听地址（等同 `ATTUNE_DOCS_LISTEN`） |
| `ATTUNE_DOCS_LISTEN` | `0.0.0.0:8200` | 同 `BIND_ADDR`，两者均可使用 |
| `DATABASE_URL` | `sqlite:///data/attune-docs.db` | SQLite 数据库路径（使用 `file:///绝对路径` 或相对路径） |
| `SQLITE_PATH` | 由 `DATABASE_URL` 推导 | 兼容旧配置的 SQLite 文件路径 |
| `OLLAMA_URL` | `http://localhost:11434` | Ollama API 地址（embedding 模型） |
| `EMBED_MODEL` | `bge-m3` | Ollama embedding 模型名 |
| `MINERU_URL` | 未设置 | MinerU sidecar 地址（full tier 必填；未设置时回退 lite 路径） |
| `MAX_OCR_CONCURRENCY` | `2` | 并发 OCR 任务上限 |
| `PDFIUM_DYNAMIC_LIB_PATH` | 未设置 | libpdfium.so 绝对路径（未设置时 PDF 解析会报错） |
| `LOG_LEVEL` | `info` | 日志级别（trace/debug/info/warn/error） |

---

## 运行测试

### Rust 单元测试

```bash
# 全量测试（跳过需要 fixture 文件的集成测试）
cargo test --workspace

# 含集成测试（需要 tests/fixtures/ 中的 PII 数据文件，默认 gitignored）
cargo test --workspace -- --include-ignored
```

> **PII fixture 注意**：`crates/attune-docs-core/tests/fixtures/` 中的 `.pdf` 和 `.docx`
> 文件包含测试用真实文档，已加入 `.gitignore`。CI 中通过独立 secret 管理的测试 fixture
> 提供；本地开发可联系项目管理员获取脱敏版。

### Token ≈ Char 近似值说明

当前 chunker 使用 `chars().count() / 4` 估算 token 数（适用中英文混合场景）。这是
有意的设计简化，精确 tokenizer 会引入 sentencepiece 依赖，超出 v1.0 成本预算。
如需精确 token 计数，可替换 `crates/attune-docs-core/src/chunker/` 中的估算函数。

### Python 客户端测试

```bash
cd clients/python
python -m pytest tests/ -q
# 需要预先激活 venv：source .venv/bin/activate
```

### TypeScript 客户端测试

```bash
cd clients/typescript
npx vitest run
```

---

## Lite vs Full 部署决策

| 维度 | Lite | Full |
|------|------|------|
| 容器数量 | 1 | 2（+ MinerU sidecar） |
| 向量存储 | sqlite-vec | sqlite-vec（v1.1 加 Weaviate） |
| OCR 引擎 | PP-OCRv4（内置 ONNX） | PP-OCRv4 + MinerU（自动 fallback） |
| 推荐场景 | 小团队 / 开发环境 / 普通 PDF | 学术论文 / 复杂表格 / 公式识别 |
| MinerU 检测 | 不可用时直接用内置 OCR | 健康探针通过后才路由 |
| 资源需求 | 低（2 vCPU / 2G RAM） | 中（4 vCPU / 4G RAM，首次下载 MinerU 模型 ~3GB） |

**选择逻辑**：默认从 Lite 开始。当遇到公式识别失败或复杂表格错乱时，切换到 Full。

### MinerU 升级策略

Full compose 中 MinerU 版本已 pin（当前 `0.10.1`）。升级步骤：

1. 在 staging 环境测试新版本 API 兼容性（`/health` 端点 + 解析响应格式）
2. 更新 `docker/full/docker-compose.yml` 中的 image tag
3. 更新本文档中的版本号
4. 提交带 `chore(deploy): bump mineru to X.Y.Z` 的 commit

---

## 客户端生成说明

Python 和 TypeScript 客户端基于 `openapi.yaml` 手工维护（非自动生成）。

若修改了服务端 API（`crates/attune-docs-server/src/routes/`），需同步更新：
1. `openapi.yaml` — OpenAPI 规范
2. `clients/python/attune_docs/` — Python 客户端
3. `clients/typescript/src/` — TypeScript 客户端
4. 对应测试文件

---

## 从 attune-enterprise docling-serve 迁移

`attune-enterprise` 的 `backend/services/docling_client.py` 提供了一个**非破坏性 opt-in shim**。

### 激活方式

在 `attune-enterprise` 的 docker-compose 中设置环境变量：

```yaml
environment:
  ATTUNE_DOCS_URL: "http://attune-docs-server:8200"
```

设置后，`docling_client.py` 会将文档解析请求路由到 attune-docs-server，而非原 Docling-Serve。

### 未设置时的行为

`ATTUNE_DOCS_URL` 未设置时，所有原有行为完全不变（`DoclingClient`、`parse_pdf_async`、
`parse_pdf_file_async`、`get_docling` 函数均正常运行，不依赖 `attune_docs` 包）。

### 安装 Python SDK

```bash
cd /data/company/project/attune-docs/clients/python
pip install -e .
```

### 返回值格式兼容说明

shim 层保持与原 `parse_document()` 相同的返回结构：

```python
{"markdown": str, "tables": list, "metadata": dict}
```

callers（`apps/cases/analyzers/pdf.py`、`apps/knowledge/views.py`）无需修改。

---

## 已知限制与路线图

### v1.0 已知限制

- **`/v1/search` score 语义**：`score` 字段的计算方式为 `1 - distance`，其中 `distance` 是 sqlite-vec 默认的 L2/Euclidean 距离。因此 `score` 是单调的"越近越大"排名值（higher = 更接近查询向量），**不是** 归一化的余弦相似度，**不保证**值域在 `[0, 1]` 内——当 L2 距离大于 1 时 score 可能为负。调用方使用 `threshold` 参数时，应基于实际观测的 score 分布进行校准，而不能假设其具有余弦相似度的 0–1 语义。v1.1 计划引入归一化余弦相似度选项。

- **EPUB 解析**：v1.0 返回 `FormatUnsupported` 错误。EPUB 解析（基于 `epub` crate 解压 XHTML + 复用 HtmlParser）计划在 v1.1 实现。
- **WeaviateStore**：v1.0 仅实现 `SqliteVecStore`。`WeaviateStore`（分布式多节点企业版）计划在 v1.1 引入，`VectorStore` trait 已预留扩展点，届时可无缝切换。
- **MinerU GPU 支持**：当前 full compose 使用 `MINERU_DEVICE=cpu`。GPU 加速（`cuda`）需要额外的 NVIDIA runtime 配置，按需添加 `docker-compose.gpu.yml` override。
- **Token 计数近似**：chunker 使用 `chars / 4` 估算，非精确 tokenizer（见测试章节说明）。
- **sqlite-vec vendor patch**：`vendor/sqlite-vec/` 携带的是 sqlite-vec **0.1.10-alpha.4 的补丁版本**，而非 crates.io 原始 tarball（原始包缺少 `sqlite-vec-diskann.c` / `sqlite-vec-rescore.c`；补丁用 `#if SQLITE_VEC_ENABLE_DISKANN` / `#if SQLITE_VEC_ENABLE_RESCORE` 宏将这两个扩展模块门控为默认关闭）。此补丁通过 `Cargo.toml` 的 `[patch.crates-io]` 机制注入。**升级 sqlite-vec 时必须重新校验并重新应用该 vendor patch**，同时更新对应的 LICENSE 文件。详见 `vendor/sqlite-vec/PATCH-NOTES.md`。

### v1.1 计划

- WeaviateStore（企业版分布式向量存储）
- EPUB 解析支持
- 异步队列（多文档并发解析）
- Weaviate 向量索引迁移工具
