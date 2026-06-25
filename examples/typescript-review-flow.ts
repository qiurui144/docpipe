import { readFile } from "node:fs/promises";
import { basename } from "node:path";
import { DocpipeClient } from "../clients/typescript/src/client.js";

const path = process.argv[2];
if (!path) {
  throw new Error("usage: DOCPIPE_URL=http://localhost:8200 tsx examples/typescript-review-flow.ts <pdf|docx|html>");
}

const baseUrl = process.env.DOCPIPE_URL ?? "http://localhost:8200";
const collection = process.env.DOCPIPE_COLLECTION ?? "default";
const client = new DocpipeClient(baseUrl);

const bytes = await readFile(path);
const file = new File([bytes], basename(path));

const health = await client.health();
console.log("health:", health.status, health.ram_tier);

const result = await client.ingest(file, {
  collection,
  config: { ocr: true, table_structure: false },
});

if (!("doc_id" in result)) {
  throw new Error(`unexpected async job response: ${result.job_id}`);
}

console.log("ingested:", result.doc_id, result.chunk_count);

const hits = await client.search("合同 审阅 风险", { collection, topK: 5 });
console.log("hits:", hits.length);

if (hits.length > 0) {
  const annotation = await client.annotate({
    doc_id: result.doc_id,
    original_text: hits[0].text,
    content: "需要人工复核该段内容。",
    label: "review-needed",
    color: "#f59e0b",
    locator: { page_num: 1, char_offset: 0 },
    source: "ai",
  });
  console.log("annotation:", annotation.item_id);
}
