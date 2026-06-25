from __future__ import annotations

import os
import sys

from docpipe import DocpipeClient


def main() -> None:
    if len(sys.argv) < 2:
        raise SystemExit("usage: DOCPIPE_URL=http://localhost:8200 python python_review_flow.py <pdf|docx|html>")

    path = sys.argv[1]
    base_url = os.environ.get("DOCPIPE_URL", "http://localhost:8200")
    collection = os.environ.get("DOCPIPE_COLLECTION", "default")

    with DocpipeClient(base_url) as client:
        health = client.health()
        print("health:", health["status"], health.get("ram_tier"))

        result = client.ingest(
            path,
            collection=collection,
            config={"ocr": True, "table_structure": False},
        )
        print("ingested:", result.doc_id, result.chunk_count)

        hits = client.search("合同 审阅 风险", collection=collection, top_k=5)
        print("hits:", len(hits))

        if hits:
            first = hits[0]
            annotation = client.annotate(
                doc_id=result.doc_id,
                original_text=first["text"],
                content="需要人工复核该段内容。",
                label="review-needed",
                color="#f59e0b",
                locator={"page_num": 1, "char_offset": 0},
                source="ai",
            )
            print("annotation:", annotation["item_id"])


if __name__ == "__main__":
    main()
