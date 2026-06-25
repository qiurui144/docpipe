#!/usr/bin/env bash
set -euo pipefail

DOCPIPE_URL="${DOCPIPE_URL:-http://localhost:8200}"
FILE="${1:?usage: DOCPIPE_URL=http://localhost:8200 $0 <pdf|docx|html> [collection]}"
COLLECTION="${2:-default}"

echo "health:"
curl -fsS "$DOCPIPE_URL/v1/health"
echo

echo "ingest:"
INGEST_JSON=$(
  curl -fsS -X POST "$DOCPIPE_URL/v1/ingest" \
    -F "file=@${FILE}" \
    -F "config={\"collection\":\"${COLLECTION}\",\"ocr\":true,\"table_structure\":false,\"async\":false}"
)
echo "$INGEST_JSON"
echo

DOC_ID=$(printf '%s' "$INGEST_JSON" | sed -n 's/.*"doc_id":"\([^"]*\)".*/\1/p')

echo "search:"
curl -fsS -X POST "$DOCPIPE_URL/v1/search" \
  -H "Content-Type: application/json" \
  -d "{\"query\":\"合同 审阅 风险\",\"collection\":\"${COLLECTION}\",\"top_k\":5}"
echo

echo "document:"
curl -fsS "$DOCPIPE_URL/v1/documents/$DOC_ID?collection=$COLLECTION"
echo
