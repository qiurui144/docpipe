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
});
