#!/usr/bin/env bash
# PII NER 多档模型 F1 评测编排脚本
#
# !! 仅在指定目标机器上运行，不在开发主机执行（§1.6）!!
# !! 本地 3B 层（Ollama qwen2.5:3b）启动前需要用户明确授权（§1.3）!!
# !! API key 通过 /tmp/secrets-*/key.env 注入（§1.4），本脚本不硬编码任何密钥 !!
#
# 用法：
#   bash scripts/pii-eval.sh [--seeds N]
#
# 环境变量（每档均可覆盖）：
#   LOCAL_BASE_URL   LOCAL_MODEL
#   WEAK_BASE_URL    WEAK_MODEL    WEAK_API_KEY
#   STRONG_BASE_URL  STRONG_MODEL  STRONG_API_KEY
#   DOCPIPE_PII_API_KEY  (weak/strong 共享 fallback)
#   SEEDS            FLOOR         (汇总判定参数)

set -euo pipefail

# ── 加载 secrets（§1.4）─────────────────────────────────────────────────────
for f in /tmp/secrets-*/key.env; do
  # shellcheck source=/dev/null
  [ -f "$f" ] && . "$f"
done

# ── 档位默认值（可由环境变量覆盖）──────────────────────────────────────────
LOCAL_BASE_URL="${LOCAL_BASE_URL:-http://localhost:11434/v1}"
LOCAL_MODEL="${LOCAL_MODEL:-qwen2.5:3b}"

WEAK_BASE_URL="${WEAK_BASE_URL:-}"
WEAK_MODEL="${WEAK_MODEL:-deepseek-flash}"
# weak key: 优先用专属 key，fallback 到通用 key
WEAK_API_KEY="${WEAK_API_KEY:-${DOCPIPE_PII_API_KEY:-}}"

STRONG_BASE_URL="${STRONG_BASE_URL:-}"
STRONG_MODEL="${STRONG_MODEL:-deepseek-v4}"
# strong key: 优先用专属 key，fallback 到通用 key
STRONG_API_KEY="${STRONG_API_KEY:-${DOCPIPE_PII_API_KEY:-}}"

SEEDS="${SEEDS:-3}"
FLOOR="${FLOOR:-0.60}"

# ── 解析 CLI 参数 ────────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
  case "$1" in
    --seeds)
      SEEDS="$2"
      shift 2
      ;;
    *)
      echo "error: unknown argument: $1" >&2
      exit 1
      ;;
  esac
done

# ── 输出目录 ─────────────────────────────────────────────────────────────────
TS=$(date +%Y%m%d_%H%M%S)
OUT="reports/runs/${TS}"
mkdir -p "${OUT}"

echo "=== PII NER Eval  ts=${TS}  seeds=${SEEDS} ===" >&2
echo "Output dir: ${OUT}" >&2

# ── 辅助：运行单档评测 ────────────────────────────────────────────────────────
# run_tier <tier_name> <base_url> <model> <api_key>
run_tier() {
  local tier="$1"
  local base_url="$2"
  local model="$3"
  local api_key="$4"
  local out_json="${OUT}/pii-eval-${tier}.json"
  local out_log="${OUT}/pii-eval-${tier}.log"

  echo "--- tier: ${tier}  model: ${model} ---" >&2

  export DOCPIPE_PII_BASE_URL="${base_url}"
  export DOCPIPE_PII_MODEL="${model}"
  export DOCPIPE_PII_API_KEY="${api_key}"

  if cargo run --release --example pii_ner_eval -- --seeds "${SEEDS}" \
      > "${out_json}" 2>"${out_log}"; then
    echo "  OK: ${out_json}" >&2
  else
    local exit_code=$?
    echo "  FAILED (exit ${exit_code}): see ${out_log}" >&2
    # 写入失败标记，以便汇总时不伪造结果
    echo '{"tier_status":"FAILED"}' > "${out_json}"
  fi
}

# ── 档位 1：本地（Ollama qwen2.5:3b）────────────────────────────────────────
LOCAL_BLOCKED=0
if curl -fsS http://localhost:11434/api/tags >/dev/null 2>&1; then
  run_tier "local" "${LOCAL_BASE_URL}" "${LOCAL_MODEL}" ""
else
  LOCAL_BLOCKED=1
  echo "--- tier: local  SKIPPED ---" >&2
  echo "  local tier SKIPPED — Ollama not running." >&2
  echo "  Starting Ollama + pulling qwen2.5:3b needs explicit approval (§1.3)." >&2
  echo "  To enable: obtain approval, start Ollama manually, then re-run this script." >&2
  echo '{"tier_status":"BLOCKED","reason":"Ollama not running; explicit approval required per §1.3"}' \
    > "${OUT}/pii-eval-local.json"
fi

# ── 档位 2：weak-cloud ────────────────────────────────────────────────────────
if [ -n "${WEAK_BASE_URL}" ]; then
  run_tier "weak-cloud" "${WEAK_BASE_URL}" "${WEAK_MODEL}" "${WEAK_API_KEY}"
else
  echo "--- tier: weak-cloud  SKIPPED (WEAK_BASE_URL not set) ---" >&2
  echo '{"tier_status":"SKIPPED","reason":"WEAK_BASE_URL not set"}' \
    > "${OUT}/pii-eval-weak-cloud.json"
fi

# ── 档位 3：strong-cloud ─────────────────────────────────────────────────────
if [ -n "${STRONG_BASE_URL}" ]; then
  run_tier "strong-cloud" "${STRONG_BASE_URL}" "${STRONG_MODEL}" "${STRONG_API_KEY}"
else
  echo "--- tier: strong-cloud  SKIPPED (STRONG_BASE_URL not set) ---" >&2
  echo '{"tier_status":"SKIPPED","reason":"STRONG_BASE_URL not set"}' \
    > "${OUT}/pii-eval-strong-cloud.json"
fi

# 清除导出的凭据环境变量，避免后续子进程继承
unset DOCPIPE_PII_BASE_URL DOCPIPE_PII_MODEL DOCPIPE_PII_API_KEY

# ── 汇总：计算各档 f1_mean，输出 §4.5D 判定 ──────────────────────────────────
echo "" >&2
echo "=== Generating summary ===" >&2

SUMMARY="${OUT}/SUMMARY.md"

{
  echo "# PII NER Eval Summary"
  echo ""
  echo "ts: ${TS}  seeds: ${SEEDS}"
  echo ""
  echo "## Per-tier F1"
  echo ""

  F1_VALS=""  # 空格分隔的已完成档位 f1_mean

  for tier in local weak-cloud strong-cloud; do
    json="${OUT}/pii-eval-${tier}.json"
    if [ ! -f "${json}" ]; then
      echo "- **${tier}**: file missing"
      continue
    fi

    # 尝试提取 tier_status（跳过档位）
    tier_status=""
    if command -v jq >/dev/null 2>&1; then
      tier_status=$(jq -r '.tier_status // empty' "${json}" 2>/dev/null || true)
    else
      tier_status=$(awk -F'"' '/"tier_status"/{print $4}' "${json}" || true)
    fi

    if [ -n "${tier_status}" ]; then
      reason=""
      if command -v jq >/dev/null 2>&1; then
        reason=$(jq -r '.reason // ""' "${json}" 2>/dev/null || true)
      fi
      echo "- **${tier}**: ${tier_status}${reason:+ — ${reason}}"
      continue
    fi

    # 提取 f1_mean
    f1_mean=""
    if command -v jq >/dev/null 2>&1; then
      f1_mean=$(jq -r '.f1_mean' "${json}" 2>/dev/null || true)
    else
      f1_mean=$(awk -F'[: ,]' '/"f1_mean"/{for(i=1;i<=NF;i++) if($i~/^[0-9]/) {print $i; break}}' "${json}" || true)
    fi

    if [ -z "${f1_mean}" ] || [ "${f1_mean}" = "null" ]; then
      echo "- **${tier}**: parse error — see ${json}"
      continue
    fi

    echo "- **${tier}**: f1_mean=${f1_mean}"
    F1_VALS="${F1_VALS} ${f1_mean}"
  done

  echo ""
  echo "## §4.5D Verdict"
  echo ""

  # 需要至少两个有效档位才能计算 spread
  f1_count=$(echo "${F1_VALS}" | wc -w | tr -d ' ')
  if [ "${f1_count}" -lt 2 ]; then
    echo "**INCONCLUSIVE** — fewer than 2 tiers produced results; cannot compute spread."
    echo ""
    echo "> synthetic corpus → advisory WARN, not a PASS; curated corpus needed for a hard min-tier claim."
  else
    # 使用 awk 计算 max、min、spread，避免 bc/python 依赖
    read -r f1_min f1_max <<< "$(echo "${F1_VALS}" | tr ' ' '\n' | grep -v '^$' | \
      awk 'BEGIN{min=999;max=-999} {v=$1+0; if(v<min)min=v; if(v>max)max=v} END{print min, max}')"

    spread=$(awk "BEGIN{printf \"%.6f\", ${f1_max} - ${f1_min}}")

    echo "spread (max-min f1_mean): ${spread}"
    echo "FLOOR: ${FLOOR}"
    echo ""

    # 判定（awk 做浮点比较）
    if awk "BEGIN{exit !(${spread} <= 0.15)}"; then
      echo "**ALL-MODEL OK (no min-tier warning)**"
    else
      # 找出 f1_mean >= FLOOR 的最低档位
      min_tier="NONE"
      for tier in local weak-cloud strong-cloud; do
        json="${OUT}/pii-eval-${tier}.json"
        [ -f "${json}" ] || continue

        tier_status=""
        if command -v jq >/dev/null 2>&1; then
          tier_status=$(jq -r '.tier_status // empty' "${json}" 2>/dev/null || true)
        fi
        [ -n "${tier_status}" ] && continue

        f1_mean=""
        if command -v jq >/dev/null 2>&1; then
          f1_mean=$(jq -r '.f1_mean' "${json}" 2>/dev/null || true)
        fi
        [ -z "${f1_mean}" ] || [ "${f1_mean}" = "null" ] && continue

        if awk "BEGIN{exit !(${f1_mean} + 0 >= ${FLOOR} + 0)}"; then
          min_tier="${tier}"
          break
        fi
      done

      if [ "${min_tier}" = "NONE" ]; then
        echo "**BELOW FLOOR** — no tier achieved f1_mean >= ${FLOOR}"
      else
        echo "**MIN-TIER REQUIRED: ${min_tier}** (first tier with f1_mean >= ${FLOOR})"
      fi
    fi

    echo ""
    echo "> synthetic corpus → advisory **WARN**, not a PASS; curated corpus needed for a hard min-tier claim."
  fi

  if [ "${LOCAL_BLOCKED}" -eq 1 ]; then
    echo ""
    echo "> local tier was BLOCKED (Ollama not running). See §1.3 for approval process."
  fi
} > "${SUMMARY}"

echo "Summary written: ${SUMMARY}" >&2
cat "${SUMMARY}" >&2
