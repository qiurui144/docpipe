"""attune-docs HTTP 契约的 pydantic 镜像（spec §5）。"""
from __future__ import annotations

from typing import Any
from pydantic import BaseModel


class BBox(BaseModel):
    x: int
    y: int
    w: int
    h: int


class TextBlock(BaseModel):
    text: str
    bbox: BBox
    confidence: float


class Table(BaseModel):
    rows: list[list[str]]


class PageContent(BaseModel):
    page_num: int
    text: str
    blocks: list[TextBlock] = []
    tables: list[Table] = []


class ParsedDocument(BaseModel):
    doc_id: str
    format: str
    page_count: int
    ocr_used: bool
    backend: str
    pages: list[PageContent]
    warnings: list[str] = []


class SearchResult(BaseModel):
    chunk_id: str
    text: str
    score: float
    metadata: dict[str, Any] = {}
