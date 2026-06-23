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

### ONNX 模型自动下载

PP-OCRv4 ONNX 模型在首次使用时自动下载到：

```
~/.local/share/attune-docs/models/ppocr/
```

容器内对应 volume 路径：`/root/.local/share/attune-docs/models/`（已在 compose 文件中挂载）。

首次启动会有几十秒的模型下载延迟。后续重启从 volume 缓存读取，无需重下。

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

- **EPUB 解析**：v1.0 返回 `FormatUnsupported` 错误。EPUB 解析（基于 `epub` crate 解压 XHTML + 复用 HtmlParser）计划在 v1.1 实现。
- **WeaviateStore**：v1.0 仅实现 `SqliteVecStore`。`WeaviateStore`（分布式多节点企业版）计划在 v1.1 引入，`VectorStore` trait 已预留扩展点，届时可无缝切换。
- **MinerU GPU 支持**：当前 full compose 使用 `MINERU_DEVICE=cpu`。GPU 加速（`cuda`）需要额外的 NVIDIA runtime 配置，按需添加 `docker-compose.gpu.yml` override。
- **Token 计数近似**：chunker 使用 `chars / 4` 估算，非精确 tokenizer（见测试章节说明）。

### v1.1 计划

- WeaviateStore（企业版分布式向量存储）
- EPUB 解析支持
- 异步队列（多文档并发解析）
- Weaviate 向量索引迁移工具
