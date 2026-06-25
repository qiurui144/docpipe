import type {
  DocumentInfo,
  HealthResponse,
  IngestResult,
  Job,
  ParsedDocument,
  PiiResult,
  SearchResult,
} from "./types.js";

export class DocpipeClient {
  private baseUrl: string;

  constructor(baseUrl: string) {
    this.baseUrl = baseUrl.replace(/\/$/, "");
  }

  private async post(path: string, body: unknown): Promise<unknown> {
    const r = await fetch(`${this.baseUrl}${path}`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    });
    return this.handle(r);
  }

  private async handle(r: Response): Promise<unknown> {
    let json: unknown;
    try {
      json = await r.json();
    } catch {
      // 非 JSON 错误体（如反代返回的 413/502 纯文本/HTML）
    }
    if (!r.ok) {
      const code = (json as { error?: string } | undefined)?.error ?? `http-${r.status}`;
      throw new Error(code);
    }
    return json;
  }

  async health(): Promise<HealthResponse> {
    const r = await fetch(`${this.baseUrl}/v1/health`);
    return this.handle(r) as Promise<HealthResponse>;
  }

  async parse(file: Blob, config?: Record<string, unknown>): Promise<ParsedDocument> {
    const form = new FormData();
    form.append("file", file);
    if (config) form.append("config", JSON.stringify(config));
    const r = await fetch(`${this.baseUrl}/v1/parse`, { method: "POST", body: form });
    return this.handle(r) as Promise<ParsedDocument>;
  }

  async ingest(file: Blob, opts: {
    config?: Record<string, unknown>;
    collection?: string;
    async?: boolean;
  } = {}): Promise<IngestResult | Job> {
    const form = new FormData();
    form.append("file", file);
    form.append("config", JSON.stringify({
      ...(opts.config ?? {}),
      collection: opts.collection ?? "default",
      async: opts.async ?? false,
    }));
    const r = await fetch(`${this.baseUrl}/v1/ingest`, { method: "POST", body: form });
    return this.handle(r) as Promise<IngestResult | Job>;
  }

  async chunk(text: string, opts: { chunkSize?: number; overlap?: number; respectHeadings?: boolean } = {}): Promise<unknown[]> {
    const json = await this.post("/v1/chunk", {
      text,
      chunk_size: opts.chunkSize ?? 512,
      overlap: opts.overlap ?? 0.2,
      respect_headings: opts.respectHeadings ?? true,
    }) as { chunks: unknown[] };
    return json.chunks;
  }

  async embed(texts: string[], model?: string): Promise<number[][]> {
    const json = await this.post("/v1/embed", { texts, model }) as { embeddings: number[][] };
    return json.embeddings;
  }

  async search(query: string, opts: { collection?: string; topK?: number; threshold?: number } = {}): Promise<SearchResult[]> {
    const json = await this.post("/v1/search", {
      query,
      collection: opts.collection ?? "default",
      top_k: opts.topK ?? 5,
      threshold: opts.threshold,
    }) as { results: SearchResult[] };
    return json.results;
  }

  async annotate(req: {
    doc_id: string; original_text: string; content: string; label: string; color: string;
    locator: { page_num: number; char_offset: number; bbox?: [number, number, number, number] };
    source?: string; skill_metadata?: Record<string, unknown>;
  }): Promise<{ item_id: string; text_hash: string }> {
    return this.post("/v1/annotate", { source: "ai", ...req }) as Promise<{ item_id: string; text_hash: string }>;
  }

  async listDocuments(collection = "default"): Promise<DocumentInfo[]> {
    const r = await fetch(`${this.baseUrl}/v1/documents?collection=${encodeURIComponent(collection)}`);
    const json = await this.handle(r) as { documents: DocumentInfo[] };
    return json.documents;
  }

  async getDocument(docId: string, collection = "default"): Promise<DocumentInfo> {
    const r = await fetch(`${this.baseUrl}/v1/documents/${encodeURIComponent(docId)}?collection=${encodeURIComponent(collection)}`);
    return this.handle(r) as Promise<DocumentInfo>;
  }

  async deleteDocument(docId: string, collection = "default"): Promise<{ deleted: boolean; doc_id: string }> {
    const r = await fetch(`${this.baseUrl}/v1/documents/${encodeURIComponent(docId)}?collection=${encodeURIComponent(collection)}`, {
      method: "DELETE",
    });
    return this.handle(r) as Promise<{ deleted: boolean; doc_id: string }>;
  }

  async detectPii(req: { text?: string; docId?: string; collection?: string; types?: string[]; redact?: boolean; annotate?: boolean }): Promise<PiiResult> {
    const body: Record<string, unknown> = { collection: req.collection ?? "default", redact: req.redact ?? false, annotate: req.annotate ?? false };
    if (req.text !== undefined) body.text = req.text;
    if (req.docId !== undefined) body.doc_id = req.docId;
    if (req.types !== undefined) body.types = req.types;
    const r = await fetch(`${this.baseUrl}/v1/detect-pii`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) });
    return this.handle(r) as Promise<PiiResult>;
  }

  async getJob(jobId: string): Promise<Job> {
    const r = await fetch(`${this.baseUrl}/v1/jobs/${encodeURIComponent(jobId)}`);
    return this.handle(r) as Promise<Job>;
  }
}
