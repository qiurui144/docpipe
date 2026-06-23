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
