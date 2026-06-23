"""attune-docs SDK Python client（attune-enterprise Django 用）。"""
from __future__ import annotations

import json
from typing import Any, Optional

import httpx

from .models import ParsedDocument


class AttuneDocsClient:
    def __init__(self, base_url: str, timeout: float = 300.0):
        self.base_url = base_url.rstrip("/")
        self._client = httpx.Client(timeout=timeout)

    def close(self) -> None:
        self._client.close()

    def __enter__(self):
        return self

    def __exit__(self, *exc) -> None:
        self.close()

    def health(self) -> dict[str, Any]:
        r = self._client.get(f"{self.base_url}/v1/health")
        r.raise_for_status()
        return r.json()

    def parse_bytes(
        self, data: bytes, filename: str = "file", config: Optional[dict] = None
    ) -> ParsedDocument:
        files = {"file": (filename, data)}
        form = {}
        if config is not None:
            form["config"] = json.dumps(config)
        r = self._client.post(f"{self.base_url}/v1/parse", files=files, data=form)
        r.raise_for_status()
        return ParsedDocument(**r.json())

    def parse(self, path: str, config: Optional[dict] = None) -> ParsedDocument:
        with open(path, "rb") as f:
            return self.parse_bytes(f.read(), filename=path.split("/")[-1], config=config)

    def chunk(
        self,
        text: str,
        chunk_size: int = 512,
        overlap: float = 0.2,
        respect_headings: bool = True,
    ) -> list[dict]:
        r = self._client.post(
            f"{self.base_url}/v1/chunk",
            json={
                "text": text,
                "chunk_size": chunk_size,
                "overlap": overlap,
                "respect_headings": respect_headings,
            },
        )
        r.raise_for_status()
        return r.json()["chunks"]

    def embed(self, texts: list[str], model: Optional[str] = None) -> list[list[float]]:
        body: dict[str, Any] = {"texts": texts}
        if model is not None:
            body["model"] = model
        r = self._client.post(f"{self.base_url}/v1/embed", json=body)
        r.raise_for_status()
        return r.json()["embeddings"]

    def search(
        self,
        query: str,
        collection: str = "default",
        top_k: int = 5,
        threshold: Optional[float] = None,
    ) -> list[dict]:
        body: dict[str, Any] = {"query": query, "collection": collection, "top_k": top_k}
        if threshold is not None:
            body["threshold"] = threshold
        r = self._client.post(f"{self.base_url}/v1/search", json=body)
        r.raise_for_status()
        return r.json()["results"]

    def annotate(
        self,
        doc_id: str,
        original_text: str,
        content: str,
        label: str,
        color: str,
        locator: dict,
        source: str = "ai",
        skill_metadata: Optional[dict] = None,
    ) -> dict:
        body: dict[str, Any] = {
            "doc_id": doc_id,
            "original_text": original_text,
            "content": content,
            "label": label,
            "color": color,
            "locator": locator,
            "source": source,
        }
        if skill_metadata is not None:
            body["skill_metadata"] = skill_metadata
        r = self._client.post(f"{self.base_url}/v1/annotate", json=body)
        r.raise_for_status()
        return r.json()
