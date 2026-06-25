import json as _json
import respx
import httpx
from docpipe.client import DocpipeClient
from docpipe.models import DocumentInfo, IngestResult, Job, ParsedDocument


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


@respx.mock
def test_ingest_and_document_lifecycle():
    ingest_payload = {
        "doc_id": "d1",
        "collection": "cases",
        "chunk_count": 1,
        "chunk_ids": ["d1:c1"],
        "backend": "text-layer",
        "ocr_used": False,
    }
    doc_payload = {
        "doc_id": "d1",
        "collection": "cases",
        "filename": "x.html",
        "format": "html",
        "page_count": 1,
        "chunk_count": 1,
        "created_at": "2026-06-24T00:00:00Z",
    }
    respx.post("http://docs/v1/ingest").mock(return_value=httpx.Response(200, json=ingest_payload))
    respx.get("http://docs/v1/documents").mock(return_value=httpx.Response(200, json={"documents": [doc_payload]}))
    respx.get("http://docs/v1/documents/d1").mock(return_value=httpx.Response(200, json=doc_payload))
    respx.delete("http://docs/v1/documents/d1").mock(return_value=httpx.Response(200, json={"deleted": True, "doc_id": "d1"}))

    c = DocpipeClient("http://docs")
    ingested = c.ingest_bytes(b"<html><body>hi</body></html>", filename="x.html", collection="cases")
    assert isinstance(ingested, IngestResult)
    assert ingested.doc_id == "d1"
    assert isinstance(c.list_documents("cases")[0], DocumentInfo)
    assert c.get_document("d1", "cases").filename == "x.html"
    assert c.delete_document("d1", "cases")["deleted"] is True


@respx.mock
def test_async_ingest_returns_job_and_get_job():
    queued = {"job_id": "j1", "status": "queued", "created_at": "now", "result": None, "error": None}
    done = {
        "job_id": "j1",
        "status": "done",
        "created_at": "now",
        "result": {
            "doc_id": "d1",
            "collection": "default",
            "chunk_count": 1,
            "chunk_ids": ["d1:c1"],
            "backend": "kreuzberg",
            "ocr_used": True,
        },
        "error": None,
    }
    respx.post("http://docs/v1/ingest").mock(return_value=httpx.Response(202, json=queued))
    respx.get("http://docs/v1/jobs/j1").mock(return_value=httpx.Response(200, json=done))

    c = DocpipeClient("http://docs")
    job = c.ingest_bytes(b"%PDF-1.7 fake", filename="x.pdf", async_=True)
    assert isinstance(job, Job)
    assert job.status == "queued"
    assert c.get_job("j1").result.doc_id == "d1"
