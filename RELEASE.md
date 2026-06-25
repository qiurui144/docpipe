# docpipe Release Notes

## v1.0.0 (2026-06-25)

### Highlights

#### Async ingest / documents / jobs API
The document pipeline now exposes a full async-ingest workflow and a document registry:

- `POST /v1/ingest` accepts `"async": true` — returns a `Job` object immediately; the
  actual parse/OCR/chunk/embed/store runs in a semaphore-bounded background queue.
- `GET /v1/jobs/{job_id}` — poll for `queued | running | done | failed` status; `done`
  carries the full `IngestResult`.
- `GET /v1/documents` — list all ingested documents with metadata (format, page count,
  chunk count, created timestamp).
- `GET /v1/documents/{doc_id}` — fetch metadata for a single document.
- `DELETE /v1/documents/{doc_id}` — remove a document and all its vector chunks.

#### PII detection (`POST /v1/detect-pii`)
New endpoint for detecting and optionally redacting or annotating personally identifiable
information in text or in a previously ingested document.

**Supported PII types:**

| Type | Detection method | Examples |
|---|---|---|
| `id_card` | Regex | 18-digit Chinese resident ID |
| `phone` | Regex | mainland-CN mobile numbers |
| `email` | Regex | standard RFC-5322 addresses |
| `bank_card` | Regex | 13–19 digit Luhn-valid card numbers |
| `plate` | Regex | mainland-CN vehicle plates |
| `ipv4` | Regex | IPv4 dotted-decimal addresses |
| `person` | LLM NER | names of individuals |
| `address` | LLM NER | physical street / location addresses |
| `org` | LLM NER | organisation / company names |

Regex types (id_card, phone, email, bank_card, plate, ipv4) run deterministically with no
external dependencies. LLM NER types (person, address, org) require an OpenAI-compatible
endpoint configured via `DOCPIPE_PII_BASE_URL` / `DOCPIPE_PII_MODEL` (default: `deepseek-v4`)
/ `DOCPIPE_PII_API_KEY`; when the endpoint is absent or a sub-3B local model is detected,
LLM NER is auto-disabled and a warning is returned (regex types still work).

**Optional parameters:**

- `redact: true` — replaces each detected entity with a stable placeholder (e.g. `[EMAIL_0]`)
  and returns `redacted_text` + `mapping` (placeholder → original) for reversible de-identification.
- `annotate: true` — persists each entity as an annotation via the existing `AnnotatableItem`
  model; requires `doc_id` (text-only input returns a warning). Annotations carry page-local
  `char_offset` derived from chunk offsets and the entity's position within the chunk text.

**SDK methods:** `detect_pii(...)` (Python) / `detectPii(...)` (TypeScript).

### Breaking Changes

None.

### Migration

This release is fully additive. No configuration changes are required to existing deployments.
The new `/v1/detect-pii` endpoint is opt-in; existing `/v1/ingest`, `/v1/parse`, `/v1/search`,
and `/v1/annotate` calls are unchanged.

### Known Limitations

- **LLM NER requires a capable model tier.** The `person`, `address`, and `org` types use an
  LLM NER call against an OpenAI-compatible endpoint. Weak local 3B-class models (e.g. qwen2.5:3b,
  phi3:mini) auto-disable LLM NER — regex types continue to work. The required multi-seed,
  multi-tier (weak-local / weak-cloud / strong-cloud) real-LLM F1 evaluation is **PENDING** and
  must be run on the designated target machine before the minimum-recommended model tier claim is
  finalised. Until then, treat strong-cloud (deepseek-v4 / GPT-4o / Claude Sonnet class) as the
  validated tier.

- **`annotate: true` requires `doc_id`.** Calling with `text`-only input and `annotate: true`
  returns a warning and does not write annotations. The `char_offset` on each annotation is
  page-local, derived from chunk offsets and the entity's intra-chunk byte position.

- **CI gitleaks scan is PENDING first run.** The `.pre-commit-config.yaml` gitleaks hook is
  wired and passes locally; the first CI run against the pushed branch is pending.

- **Latent wildcard gap in LIKE queries.** The `delete_document` and `chunks_for_document`
  storage queries use `LIKE doc_id` against a SQLite column that expects UUID values. A
  non-UUID `doc_id` containing `_` or `%` would act as a SQL wildcard. Tracked as tech-debt;
  fix planned for v1.1 (parameterised exact-match query).

- **EPUB** parsing returns `format-unsupported` (v1.1).
- **WeaviateStore** is not implemented (v1.1).
- **PP-OCR ONNX models** must be placed manually in `~/.local/share/docpipe/models/ppocr/`;
  auto-download lands in v1.1.
- **`sqlite-vec` is vendored** (patched) to work around missing files in the upstream
  `0.1.10-alpha.4` crates.io tarball — see `vendor/sqlite-vec/PATCH-NOTES.md`.
- Search `score` is `1 − L2_distance` (monotonic; not normalised cosine similarity).
