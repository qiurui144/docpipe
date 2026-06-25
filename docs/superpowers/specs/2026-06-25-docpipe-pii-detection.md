# docpipe PII 检测 — 设计规范

> 状态:草案待评审 · 日期:2026-06-25 · 关联里程碑:v1.0 GA
> 前置事故:`bank_page1_expected.json` 真实人名泄露已于 2026-06-25 经 filter-repo 清史 + force-push 处置。本 spec 是其"治本"侧:让管线**主动识别并可脱敏** PII,且全程合成 fixture。

---

## 1. 目标定位

文档管线在 parse 之后,能识别文档正文里的**全类型个人信息(PII)**,并可选地脱敏或写入标注。解决合同 / 银行流水 / 法律文书审阅场景下的隐私痛点:用户上传含真实姓名、身份证号、账号的扫描件,需要在检索 / 导出 / 协作前先定位并遮蔽 PII。与产品定位(隐私优先的本地文档管线)对齐 —— PII 能力是隐私基座,而非外挂。

## 2. 范围边界

### 本版本(v1.0)做
- `pii::detect(text)` 核心:返回实体列表(类型 + 字符偏移 + 置信度 + 来源)
- 确定性正则类型:身份证(18 位,校验位验证)、手机号(中国大陆)、邮箱、银行卡(Luhn)、车牌、IPv4
- LLM NER 类型:人名、地址、组织机构名(弱模型自动禁用,见 §5/§8)
- 可选 `redact=true`:返回脱敏文本 + 可逆映射
- 可选 `annotate=true`:复用现有 annotator 把实体写为标注
- `POST /v1/detect-pii` 路由 + py/ts SDK 方法 + openapi 契约
- 全合成 fixture 的六类测试矩阵

### 不做(留后续 v1.x)
- 护照号 / 社保号 / 海外证件等地区性强模式(v1.1)
- 图像层 PII(印章、手写签名的视觉检测)—— 仅做 OCR 后的文本层
- 批量整库扫描 / 后台 PII 审计任务(v1.x,接 JobQueue)
- 自定义实体类型的用户配置 DSL(v1.x)

> Scope 写死:以上"不做"项不允许在实现期 silent 扩入。需要则回头改本 spec。

## 3. 架构数据流

```
输入: { text } 或 { doc_id, collection }
         │  (doc_id → 从 VectorStore 取已解析文本)
         ▼
   pii::detect(text, types?)
     ├── patterns.rs   确定性正则 (零成本, 永远可用)
     │     身份证18 / 手机 / 邮箱 / 银行卡(Luhn) / 车牌 / IPv4
     │     → 每命中产出 PiiEntity{ source = Regex, confidence = 1.0 }
     │
     └── llm.rs         LLM NER (§4.5, 可降级)
           人名 / 地址 / 组织机构
           schema-guided JSON → 验证-重试(≤3) → PiiEntity{ source = Llm, confidence }
         │
         ▼  合并 + 去重 (按 [start,end) 区间重叠, 正则优先于 LLM)
   Vec<PiiEntity>
         │
         ├── redact=true  → redact.rs: 按区间从后往前替换 → redacted_text
         │                   + mapping: { placeholder → original }  (可逆)
         │
         └── annotate=true → annotator::create_item 逐实体写标注 (locator 校验)
         ▼
输出: { entities[], redacted_text?, mapping?, annotations?[], warnings[] }
```

- **数据表**:不新增持久表。`annotate=true` 复用现有 annotator 存储路径。检测本身无状态。
- **缓存层**:无新增。LLM 调用走 §4.5 prompt 设计;不引入跨请求缓存(单次判别)。

## 4. 模块边界

| 路径 | 职责 | 依赖 |
|---|---|---|
| `crates/docpipe-core/src/pii/mod.rs` | `PiiEntity` / `PiiKind` 类型;`detect()` 编排 + 合并去重 | patterns, llm |
| `crates/docpipe-core/src/pii/patterns.rs` | 6 类确定性正则 + 校验(身份证/Luhn) | regex |
| `crates/docpipe-core/src/pii/llm.rs` | OpenAI-compat NER 后端;schema + 重试 + few-shot + 降级 | reqwest, serde |
| `crates/docpipe-core/src/pii/redact.rs` | 区间脱敏 + 可逆 mapping | — |
| `crates/docpipe-server/src/routes/pii.rs` | `POST /v1/detect-pii` handler;doc_id 取文本;错误码映射 | core::pii, store, annotator |
| `clients/python/docpipe/client.py` | `detect_pii(...)` | — |
| `clients/typescript/src/client.ts` | `detectPii(...)` | — |
| `openapi.yaml` | `/v1/detect-pii` schema | — |

每个单元可独立测试:patterns 纯函数;llm 可对 mock endpoint 测;detect 编排可对 stub 后端测;route 走 axum test。

## 5. API 契约

### POST /v1/detect-pii

请求:
```json
{
  "text": "string",            // text 与 doc_id 二选一
  "doc_id": "string",
  "collection": "default",
  "types": ["person","id_card","phone","email","bank_card","plate","ipv4","address","org"], // 可选, 缺省=全部
  "redact": false,
  "annotate": false
}
```

响应 200:
```json
{
  "entities": [
    { "kind": "id_card", "text": "...", "start": 12, "end": 30, "confidence": 1.0, "source": "regex" },
    { "kind": "person",  "text": "...", "start": 0,  "end": 3,  "confidence": 0.94, "source": "llm" }
  ],
  "redacted_text": "string|null",
  "mapping": { "[PERSON_1]": "原文" },
  "annotations": [ { "item_id": "..." } ],
  "warnings": ["llm-unavailable: name/address/org detection skipped"]
}
```

- `start`/`end`:UTF-8 字符偏移(非字节),半开区间。
- SDK 方法签名:
  - py: `detect_pii(self, text: str | None = None, *, doc_id: str | None = None, collection: str = "default", types: list[str] | None = None, redact: bool = False, annotate: bool = False) -> dict`
  - ts: `detectPii(req: { text?: string; docId?: string; collection?: string; types?: string[]; redact?: boolean; annotate?: boolean }): Promise<PiiResult>`
  - 线上 body 一律 snake_case;ts 的 camelCase 不得泄漏(沿用既有测试约束)。

## 6. 扩展点 / 插件接口

- `PiiKind` 为开放枚举式设计:新增地区性强模式 = 在 `patterns.rs` 加一个 `fn detect_<kind>(text) -> Vec<Match>` 并注册进 detector 列表,无需改 detect 编排签名。
- LLM 后端经 env 切换(`DOCPIPE_PII_BASE_URL` / `_MODEL` / `_API_KEY`),默认锁 `deepseek-v4`;换 provider 不改代码。
- redact 占位符策略可替换(`[PERSON_1]` vs `█`)经参数,默认带序号可逆。

## 7. 错误 + 边界 case

| 场景 | 行为 |
|---|---|
| `text` 与 `doc_id` 都缺 | 400 `bad-request` |
| `doc_id` 不存在 | 404 `document-not-found` |
| 空 / 纯空白文本 | 200,`entities: []`(非错误) |
| 超长文本 | 按字符窗口分块送 LLM,偏移回算到全局;正则全文跑 |
| LLM 不可用 / 超时 / 坏 JSON(重试 3 次仍失败) | **graceful 降级**:只返回正则类型 + `warnings: ["llm-unavailable: ..."]`,200 而非 5xx;**禁止 silent failure**(§5.2) |
| 身份证校验位不合法 | 不计入(降低误报);可选 `confidence` 体现 |
| Unicode / 繁简 / emoji / 全角数字 | 偏移按 char 计;必测 |
| `redact` 区间重叠 | 从后往前替换,正则区间优先 |

exit/错误码:kebab-case,与既有 route 错误风格一致。

## 8. 成本契约

- **正则类型**:零成本、纯本地、永远可用。
- **LLM 类型(人名/地址/机构)**:本地算力或云 token 成本。三层归属:
  - 强云(deepseek-v4 默认):全类型可用。
  - 弱云(gpt-4o-mini 级):全类型可用,RELEASE 标 F1 区间。
  - 弱本地(3B):**自动禁用** LLM NER,仅正则类型;UI/响应 `warnings` 明示。
- 单次判别调用,不做多轮;无跨请求缓存,成本可预测 = O(文本块数)。

## 9. 测试矩阵

| 类 | 用例(全部合成数据,§1.4) |
|---|---|
| happy | 每类型 ≥1 正样例(合成身份证含合法校验位、合成手机号、`测试甲/乙/丙` 人名) |
| edge | 空文本、纯空白、超长(>分块阈值)、跨块边界实体、全角数字、繁体姓名 |
| error | LLM endpoint 503 → 降级;LLM 返回非 JSON → 重试后降级;doc_id 不存在 |
| adversarial | 伪造身份证(校验位错,应不命中)、prompt 注入文本、SQL/XSS 串(不应误判为实体) |
| 并发 | N 并发 detect,无 race(检测无状态) |
| i18n | 中英混排、繁简、emoji 偏移正确 |

- LLM 路径多 tier(弱本地 / 弱云 / 强云)× 多 seed(N=3)跑 ≥10 case,F1 三 tier 差 ≤0.15 判兼容,否则 RELEASE 标最低 tier(§4.5D)。
- 确定性正则类型 PASS rate = 1.00 为门槛。
- 测试代码与本 spec 同 commit;**严禁任何真实 PII 进 fixture**。

## 10. 向后兼容

- 纯新增端点 + 新增 SDK 方法,不改既有 `/v1/*` 契约 → 老 client 不受影响。
- openapi `version` 已是 `1.0.0`;本特性随 v1.0 GA 发布。
- `PiiResult` 新字段(redacted_text/mapping/annotations/warnings)对不传 `redact`/`annotate` 的调用返回 null/空 → 默认行为最小。
- 无 schema 迁移(无持久化新表)。

## 11. 风险登记

| 风险 | 缓解 |
|---|---|
| LLM NER 误报/漏报(人名歧义、地址边界) | 正则优先合并;confidence 暴露;弱模型降级;多 seed 评估;RELEASE 标实测 F1 |
| 偏移计算 byte vs char 错位导致脱敏错位 | 统一 char 偏移 + 跨块回算;edge/i18n 用例锁死 |
| **再次把真实 PII 写进 fixture**(本次事故复发) | 全合成数据硬约束 + 加 gitleaks pre-commit/CI(v1.0 就绪项)拦截 |
| LLM provider 变更 / 下架 | env 切换 + 默认 deepseek-v4;降级路径保证基础可用 |
| 分块切断长实体(如长地址) | 块间重叠窗口;实体跨界时取并集 |
| 成本不可控(大文档大量 LLM 调用) | 仅人名/地址/机构走 LLM;正则先筛;文档级上限可配 |
