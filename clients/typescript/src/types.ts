// docpipe HTTP 契约类型（与 openapi.yaml 对应，spec §5）。

export interface BBox { x: number; y: number; w: number; h: number; }
export interface TextBlock { text: string; bbox: BBox; confidence: number; }
export interface Table { rows: string[][]; }
export interface PageContent { page_num: number; text: string; blocks: TextBlock[]; tables: Table[]; }
export interface ParsedDocument {
  doc_id: string; format: string; page_count: number; ocr_used: boolean;
  backend: string; pages: PageContent[]; warnings: string[];
}
export interface SearchResult { chunk_id: string; text: string; score: number; metadata: Record<string, unknown>; }
export interface HealthResponse { status: string; backends: Record<string, string>; ram_tier: string; }
export interface IngestResult {
  doc_id: string; collection: string; chunk_count: number; chunk_ids: string[];
  backend: string; ocr_used: boolean;
}
export interface DocumentInfo {
  doc_id: string; collection: string; filename?: string | null; format: string;
  page_count: number; chunk_count: number; created_at: string;
}
export interface Job {
  job_id: string; status: "queued" | "running" | "done" | "failed"; created_at: string;
  result?: IngestResult | null; error?: string | null;
}
export interface PiiEntity { kind: string; text: string; start: number; end: number; confidence: number; source: string; page_num?: number | null; }
export interface PiiResult { entities: PiiEntity[]; redacted_text?: string | null; mapping?: Record<string, string> | null; annotations?: unknown[] | null; warnings: string[]; }
