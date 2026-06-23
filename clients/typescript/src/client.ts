import type { ParsedDocument, SearchResult, HealthResponse } from "./types.js";

export class AttuneDocsClient {
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
}
