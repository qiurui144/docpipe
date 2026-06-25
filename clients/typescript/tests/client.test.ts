import { describe, it, expect, vi, beforeEach } from "vitest";
import { DocpipeClient } from "../src/client.js";

describe("DocpipeClient", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it("health parses status", async () => {
    vi.stubGlobal("fetch", vi.fn(async () =>
      new Response(JSON.stringify({ status: "ok", backends: {}, ram_tier: "lite" }), { status: 200 })
    ));
    const c = new DocpipeClient("http://docs");
    const h = await c.health();
    expect(h.status).toBe("ok");
    expect(h.ram_tier).toBe("lite");
  });

  it("search returns results array", async () => {
    vi.stubGlobal("fetch", vi.fn(async () =>
      new Response(JSON.stringify({ results: [{ chunk_id: "d:1", text: "hit", score: 0.9, metadata: {} }] }), { status: 200 })
    ));
    const c = new DocpipeClient("http://docs");
    const res = await c.search("q", { topK: 1 });
    expect(res.length).toBe(1);
    expect(res[0].chunk_id).toBe("d:1");
  });

  it("throws on error status with error code", async () => {
    vi.stubGlobal("fetch", vi.fn(async () =>
      new Response(JSON.stringify({ error: "format-unsupported", detail: "x" }), { status: 400 })
    ));
    const c = new DocpipeClient("http://docs");
    await expect(c.search("q")).rejects.toThrow("format-unsupported");
  });

  it("search sends snake_case body and camelCase does not leak", async () => {
    const fetchMock = vi.fn(async () =>
      new Response(JSON.stringify({ results: [{ chunk_id: "d:1", text: "hit", score: 0.9, metadata: {} }] }), { status: 200 })
    );
    vi.stubGlobal("fetch", fetchMock);
    const c = new DocpipeClient("http://docs");
    await c.search("q", { topK: 1 });
    const [, init] = fetchMock.mock.calls[0];
    const body = JSON.parse((init as RequestInit).body as string);
    expect(body.top_k).toBe(1);       // snake_case on the wire
    expect(body.topK).toBeUndefined(); // camelCase must NOT leak
  });

  it("throws http-N fallback on non-JSON error body", async () => {
    vi.stubGlobal("fetch", vi.fn(async () =>
      new Response("<html>502</html>", { status: 502 })
    ));
    const c = new DocpipeClient("http://docs");
    await expect(c.search("q")).rejects.toThrow("http-502");
  });

  it("ingest sends multipart config and returns result", async () => {
    const fetchMock = vi.fn(async () =>
      new Response(JSON.stringify({
        doc_id: "d1",
        collection: "cases",
        chunk_count: 1,
        chunk_ids: ["d1:c1"],
        backend: "text-layer",
        ocr_used: false,
      }), { status: 200 })
    );
    vi.stubGlobal("fetch", fetchMock);
    const c = new DocpipeClient("http://docs");
    const res = await c.ingest(new Blob(["<html>hi</html>"]), { collection: "cases" });
    expect("doc_id" in res && res.doc_id).toBe("d1");
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("http://docs/v1/ingest");
    expect((init as RequestInit).method).toBe("POST");
    expect((init as RequestInit).body).toBeInstanceOf(FormData);
  });

  it("detectPii sends snake_case doc_id and returns entities", async () => {
    const fetchMock = vi.fn(async () =>
      new Response(JSON.stringify({ entities: [{ kind: "email", text: "a@b.co", start: 0, end: 6, confidence: 1, source: "regex" }], warnings: [] }), { status: 200 }));
    vi.stubGlobal("fetch", fetchMock);
    const c = new DocpipeClient("http://docs");
    const res = await c.detectPii({ docId: "d1" });
    expect(res.entities[0].kind).toBe("email");
    const [, init] = fetchMock.mock.calls[0];
    const body = JSON.parse((init as RequestInit).body as string);
    expect(body.doc_id).toBe("d1");
    expect(body.docId).toBeUndefined();
  });

  it("documents and jobs use expected paths", async () => {
    const fetchMock = vi.fn(async (url: string, init?: RequestInit) => {
      if (url.includes("/v1/documents/d1") && init?.method === "DELETE") {
        return new Response(JSON.stringify({ deleted: true, doc_id: "d1" }), { status: 200 });
      }
      if (url.includes("/v1/documents/d1")) {
        return new Response(JSON.stringify({
          doc_id: "d1", collection: "cases", filename: "x.pdf", format: "pdf",
          page_count: 1, chunk_count: 1, created_at: "now",
        }), { status: 200 });
      }
      if (url.includes("/v1/documents")) {
        return new Response(JSON.stringify({ documents: [] }), { status: 200 });
      }
      return new Response(JSON.stringify({ job_id: "j1", status: "done", created_at: "now", result: null, error: null }), { status: 200 });
    });
    vi.stubGlobal("fetch", fetchMock);
    const c = new DocpipeClient("http://docs");
    expect(await c.listDocuments("cases")).toEqual([]);
    expect((await c.getDocument("d1", "cases")).filename).toBe("x.pdf");
    expect((await c.deleteDocument("d1", "cases")).deleted).toBe(true);
    expect((await c.getJob("j1")).status).toBe("done");
  });
});
