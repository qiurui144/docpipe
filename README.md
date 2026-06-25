# docpipe

> [中文文档](./README.zh.md)

**Self-hostable document pipeline — parse · OCR · chunk · embed · search · annotate.**

A general-purpose, industry-agnostic document-processing SDK. A pure-Rust core with a pluggable
trait system, wrapped by an HTTP server, with Python and TypeScript clients. Turn PDFs / DOCX / HTML
(text-layer **or** scanned) into structured, searchable, annotatable content — entirely on your own
infrastructure, no cloud API required.

> Status: **v1.0.0** — core pipeline + async ingest/documents/jobs API + PII detection.
> See [Known Limitations](#known-limitations) and [RELEASE.md](./RELEASE.md).

## Why

Every product that touches documents re-implements the same pipeline (parse → OCR → chunk → embed →
store → annotate), each with different quality. `docpipe` extracts that pipeline once, as a standalone
library + HTTP service, so any stack (Rust / Python / TypeScript / anything that speaks HTTP) gets the
same capabilities without re-inventing them.

## Features

- **Parsing** — PDF (text-layer auto-detected, OCR fallback), DOCX, HTML, with format auto-detection.
- **OCR** — PP-OCRv4 ONNX (via `kreuzberg-paddle-ocr`), Rust-native, no Python runtime. Reads scanned
  documents and security-watermarked pages (e.g. bank statements) that Tesseract fails on.
- **Tiered backends** — *Lite* (SQLite + in-process OCR, no extra containers) and *Full* (adds a MinerU
  sidecar for table-structure recovery, with automatic health-probe fallback to the built-in OCR).
- **Chunking** — semantic, sentence-boundary-aware sliding window with configurable overlap.
- **Embeddings** — any OpenAI/Ollama-compatible `/api/embed` endpoint.
- **Vector store** — SQLite + `sqlite-vec` (Lite); Weaviate planned (v1.1).
- **Annotation** — industry-agnostic `AnnotatableItem` + `TextLocator` with a content hash to detect
  document drift; AI and human annotations share one data model.
- **Pluggable** — `DocParser`, `OcrBackend`, `Embedder`, `VectorStore` are traits; bring your own.

## Architecture

```
            HTTP /v1/*                        Rust crate (link directly)
  Python / TS / any client  ─┐        ┌─  docpipe-core (pure library, no HTTP)
                             ▼        ▼
                      docpipe-server (axum)  ──►  parser · ocr · chunker
                                                   embedder · store · annotator
                                                        │
                                       KreuzbergBackend (PP-OCRv4 ONNX, default)
                                       MinerUBackend   (HTTP sidecar, optional)
```

| Component | Crate / package | Purpose |
|---|---|---|
| `docpipe-core` | crates/docpipe-core | Pure Rust library: traits, types, parser, OCR, chunker, embedder, store, annotator |
| `docpipe-server` | crates/docpipe-server | axum HTTP server exposing `/v1/*` |
| Python client | `docpipe-client` (PyPI) | `from docpipe import DocpipeClient` |
| TypeScript client | `@qiurui144/docpipe` (npm) | `import { DocpipeClient } from "@qiurui144/docpipe"` |

## Quick start

### Run the server

```bash
# runtime deps: a PDFium shared library + PP-OCR ONNX models — see DEVELOP.md
export PDFIUM_DYNAMIC_LIB_PATH=/path/to/libpdfium.so   # or the directory containing it
export OLLAMA_URL=http://localhost:11434
export EMBED_MODEL=bge-m3
cargo run -p docpipe-server                            # listens on 0.0.0.0:8200
```

Or with Docker:

```bash
docker compose -f docker/lite/docker-compose.yml up    # Lite tier (SQLite, no MinerU)
docker compose -f docker/full/docker-compose.yml up    # Full tier (+ MinerU sidecar)
```

### HTTP API

| Method | Path | Purpose |
|---|---|---|
| POST | `/v1/parse` | multipart file → `ParsedDocument` (text + blocks + tables) |
| POST | `/v1/ingest` | multipart file → parse/OCR/chunk/embed/store in one call |
| POST | `/v1/chunk` | text → semantic chunks |
| POST | `/v1/embed` | texts → embedding vectors |
| POST | `/v1/search` | query → nearest chunks |
| POST | `/v1/annotate` | create an annotation item |
| GET  | `/v1/documents` | list ingested documents |
| GET/DELETE | `/v1/documents/{doc_id}` | get/delete a document and its vectors |
| GET  | `/v1/jobs/{job_id}` | async ingest job status |
| POST | `/v1/detect-pii` | detect (and optionally redact/annotate) PII entities |
| GET  | `/v1/health` | backend readiness + tier |

```bash
curl -F file=@scan.pdf \
  -F 'config={"collection":"default","ocr":true,"async":false}' \
  http://localhost:8200/v1/ingest
```

Full spec: [`openapi.yaml`](./openapi.yaml).

### Call Flow

The recommended boundary is: the existing system handles **format conversion and business workflow**;
`docpipe` handles **parsing, OCR, chunking, embeddings, storage, search, and annotation locators**.
Do not pre-OCR documents in the existing system unless that OCR output is already high-quality,
traceable, and page/coordinate aware. Prefer sending PDF / DOCX / HTML directly to `/v1/ingest`.
Unsupported source formats such as DOC, RTF, or image batches should be converted to PDF/DOCX/HTML first.

Current coverage note: scanned PDFs are OCR'd page by page. Text-layer PDFs prefer the text layer.
DOCX/HTML currently extract text and tables; embedded-image OCR is the next gap to close.

### Rust (link the core directly)

```rust
use docpipe_core::{DocpipeBuilder, ParseConfig};

let sdk = DocpipeBuilder::new()
    .ocr_backend(std::sync::Arc::new(KreuzbergBackend::new()?))
    .vector_store(std::sync::Arc::new(SqliteVecStore::new("docs.db")?))
    .embedder(std::sync::Arc::new(OllamaEmbedder::new("http://localhost:11434", "bge-m3")))
    .build()?;

let parsed = sdk.parse(&bytes, ParseConfig::default()).await?;
let ids    = sdk.ingest(&parsed, "default", Some("scan.pdf"), "2026-06-25T00:00:00Z").await?;
let hits   = sdk.search("张三 2019 跨行汇款", "default", 5).await?;
```

### Python

```python
from docpipe import DocpipeClient
doc = DocpipeClient("http://localhost:8200").parse("scan.pdf")
```

More end-to-end examples are available under [`examples/`](./examples/).

### PII Detection

Detect (and optionally redact or annotate) personally identifiable information in free text
or in a previously ingested document:

```bash
# detect PII in raw text
curl -X POST http://localhost:8200/v1/detect-pii \
  -H 'Content-Type: application/json' \
  -d '{"text":"联系 张三，手机 13800138000，邮箱 zhang@example.com","redact":true}'
```

Python SDK one-liner:

```python
result = DocpipeClient("http://localhost:8200").detect_pii(
    "手机 13800138000，邮箱 zhang@example.com",
    redact=True,
)
```

TypeScript SDK one-liner:

```typescript
const result = await client.detectPii({
  text: "手机 13800138000，邮箱 zhang@example.com",
  redact: true,
});
```

Supported PII types: `id_card`, `phone`, `email`, `bank_card`, `plate`, `ipv4` (deterministic
regex); `person`, `address`, `org` (LLM NER — requires `DOCPIPE_PII_BASE_URL` / `_API_KEY`).
Set `annotate: true` together with `doc_id` to persist per-entity annotations.

## Verified

- `cargo test --workspace`: 54 passing (Linux x64 **and** Windows x64 / MSVC), clippy clean.
- Real end-to-end on a Windows Intel target: full MSVC build (ONNX + sqlite-vec + PDFium link),
  server up, and a scanned Chinese PDF OCR'd correctly through `/v1/parse` (card numbers, amounts,
  dates all extracted) via PP-OCRv4 + a real Ollama embedder.

## Known Limitations

- **EPUB** parsing and a **Weaviate** vector backend are planned for v1.1 (EPUB currently returns
  `format-unsupported`).
- **Models are provisioned manually in v1.0** — the PP-OCR ONNX models + dictionary must be placed in
  `~/.local/share/docpipe/models/ppocr/`; the server fails fast if they are absent. Auto-download lands
  in v1.1. See [DEVELOP.md](./DEVELOP.md) (note the dictionary must be **BOM-less, LF**).
- **`sqlite-vec` is vendored** (patched) under `vendor/` to work around a missing-file bug in the
  upstream `0.1.10-alpha.4` crates.io tarball — see `vendor/sqlite-vec/PATCH-NOTES.md`.
- Search `score` is `1 − distance` over `sqlite-vec`'s L2 metric (monotonic nearest-first; not a
  normalized cosine similarity).

## Development

See [DEVELOP.md](./DEVELOP.md) for the workspace layout, runtime dependencies (PDFium, ONNX models),
the env-var table, building/testing, and client maintenance.

## License

Apache-2.0 — see [LICENSE](./LICENSE). Vendored `sqlite-vec` is MIT/Apache-2.0 (see `vendor/sqlite-vec/`).
