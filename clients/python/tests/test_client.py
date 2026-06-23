import json as _json
import respx
import httpx
from docpipe.client import DocpipeClient
from docpipe.models import ParsedDocument


@respx.mock
def test_health_parses():
    respx.get("http://docs/v1/health").mock(
        return_value=httpx.Response(200, json={
            "status": "ok",
            "backends": {"kreuzberg": "ready"},
            "ram_tier": "lite",
        })
    )
    c = DocpipeClient("http://docs")
    h = c.health()
    assert h["status"] == "ok"
    assert h["ram_tier"] == "lite"


@respx.mock
def test_search_returns_results():
    respx.post("http://docs/v1/search").mock(
        return_value=httpx.Response(200, json={
            "results": [{"chunk_id": "d:1", "text": "hit", "score": 0.92, "metadata": {}}]
        })
    )
    c = DocpipeClient("http://docs")
    res = c.search("query", top_k=1)
    assert len(res) == 1
    assert res[0]["chunk_id"] == "d:1"
    body = _json.loads(respx.calls.last.request.content)
    assert body["top_k"] == 1


@respx.mock
def test_parse_roundtrips_model():
    payload = {
        "doc_id": "u1", "format": "pdf", "page_count": 1, "ocr_used": True,
        "backend": "kreuzberg",
        "pages": [{"page_num": 1, "text": "hi", "blocks": [], "tables": []}],
        "warnings": [],
    }
    respx.post("http://docs/v1/parse").mock(return_value=httpx.Response(200, json=payload))
    c = DocpipeClient("http://docs")
    doc = c.parse_bytes(b"%PDF-1.7 fake", filename="x.pdf")
    assert isinstance(doc, ParsedDocument)
    assert doc.doc_id == "u1"
    assert doc.backend == "kreuzberg"
