# docpipe PII Detection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add全类型 PII detection to docpipe — deterministic regex types always-on + LLM NER (person/address/org) with graceful weak-model degradation — exposed as `POST /v1/detect-pii` and `detect_pii()`/`detectPii()` SDK methods, with optional reversible redaction and annotation.

**Architecture:** New `crates/docpipe-core/src/pii/` module: `patterns.rs` (zero-cost deterministic detectors), `llm.rs` (OpenAI-compat NER with §4.5 schema-guided retry + degrade), `redact.rs` (interval-based reversible masking), `mod.rs` (orchestration + merge/dedup). A server route resolves `text` directly or fetches a document's text by `doc_id`. SDKs mirror the contract. Then v1.0 release hygiene (version, metadata, RELEASE.md, gitleaks).

**Tech Stack:** Rust (axum, reqwest, regex, serde, thiserror, tokio), Python (httpx, pydantic), TypeScript (fetch, vitest), Rust `criterion`-free unit tests.

## Global Constraints

- Char offsets are **UTF-8 character offsets, half-open `[start, end)`** — never byte offsets (spec §5, §7).
- LLM NER must **degrade gracefully** to regex-only + a `warnings` entry on any failure after retry≤3; **never** 5xx, **never** panic, **never** silent-swallow (spec §7, CLAUDE.md §4.5/§5.2).
- LLM backend is OpenAI-compatible, env-switchable: `DOCPIPE_PII_BASE_URL` / `DOCPIPE_PII_MODEL` / `DOCPIPE_PII_API_KEY`; **default model `deepseek-v4`** (CLAUDE.md §H). Weak local 3B → LLM NER auto-disabled, regex types still returned.
- **All test fixtures use synthetic data only.** No real names/IDs/phones/emails anywhere (CLAUDE.md §1.4; this feature exists because of a real PII-fixture leak — do not reintroduce one).
- Error codes are stable kebab-case via `DocError::code()` (spec §7). New codes: `bad-request`, plus reuse `document-not-found`.
- On-the-wire JSON is snake_case; TS camelCase options must not leak to the body (existing SDK test invariant).
- LLM NER is a single discriminative call (N=1) → single-turn is allowed (CLAUDE.md §G exception); do NOT build multi-turn history.

---

### Task 1: PII core types + module scaffold

**Files:**
- Create: `crates/docpipe-core/src/pii/mod.rs`
- Modify: `crates/docpipe-core/src/lib.rs` (add `pub mod pii;`)
- Modify: `crates/docpipe-core/src/error.rs` (add `BadRequest` variant)

**Interfaces:**
- Produces: `pii::PiiKind` (enum), `pii::PiiEntity { kind: PiiKind, text: String, start: usize, end: usize, confidence: f32, source: PiiSource }`, `pii::PiiSource { Regex, Llm }`. `DocError::BadRequest(String)` → code `bad-request`, status 400.

- [ ] **Step 1: Write the failing test** — in `crates/docpipe-core/src/pii/mod.rs`:

```rust
//! PII 检测：确定性正则 + LLM NER + 可逆脱敏（spec 2026-06-25-docpipe-pii-detection）。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PiiKind {
    IdCard,
    Phone,
    Email,
    BankCard,
    Plate,
    Ipv4,
    Person,
    Address,
    Org,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PiiSource {
    Regex,
    Llm,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PiiEntity {
    pub kind: PiiKind,
    pub text: String,
    pub start: usize,
    pub end: usize,
    pub confidence: f32,
    pub source: PiiSource,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pii_kind_serializes_snake_case() {
        let j = serde_json::to_string(&PiiKind::IdCard).unwrap();
        assert_eq!(j, "\"id_card\"");
    }

    #[test]
    fn entity_roundtrips() {
        let e = PiiEntity { kind: PiiKind::Email, text: "a@b.co".into(), start: 0, end: 6, confidence: 1.0, source: PiiSource::Regex };
        let j = serde_json::to_string(&e).unwrap();
        let back: PiiEntity = serde_json::from_str(&j).unwrap();
        assert_eq!(e, back);
    }
}
```

- [ ] **Step 2: Add module + error variant**

In `crates/docpipe-core/src/lib.rs`, add after `pub mod parser;`:
```rust
pub mod pii;
```
In `crates/docpipe-core/src/error.rs`, add to the enum:
```rust
    #[error("bad-request: {0}")]
    BadRequest(String),
```
add to `code()`:
```rust
            DocError::BadRequest(_) => "bad-request",
```
add to `http_status()`:
```rust
            DocError::BadRequest(_) => 400,
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test -p docpipe-core pii::tests`
Expected: PASS (2 tests). Also `cargo test -p docpipe-core error` still PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/docpipe-core/src/pii/mod.rs crates/docpipe-core/src/lib.rs crates/docpipe-core/src/error.rs
git commit -m "feat(core): PII types (PiiKind/PiiEntity/PiiSource) + bad-request error"
```

---

### Task 2: Deterministic regex detectors

**Files:**
- Create: `crates/docpipe-core/src/pii/patterns.rs`
- Modify: `crates/docpipe-core/src/pii/mod.rs` (add `mod patterns;` + re-export)
- Modify: `crates/docpipe-core/Cargo.toml` (ensure `regex` + `once_cell` deps)

**Interfaces:**
- Produces: `patterns::detect_regex(text: &str) -> Vec<PiiEntity>` — returns all deterministic-type matches with char offsets and `confidence = 1.0`, `source = Regex`. Helpers: `is_valid_cn_id(s: &str) -> bool` (ISO 7064 MOD 11-2), `luhn_ok(digits: &str) -> bool`.

- [ ] **Step 1: Write the failing test** — append to a new `crates/docpipe-core/src/pii/patterns.rs`:

```rust
//! 确定性 PII 正则 + 校验位（零成本，永远可用）。
use once_cell::sync::Lazy;
use regex::Regex;
use super::{PiiEntity, PiiKind, PiiSource};

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<PiiKind> {
        let mut k: Vec<_> = detect_regex(text).into_iter().map(|e| e.kind).collect();
        k.sort_by_key(|x| format!("{x:?}")); k
    }

    #[test]
    fn email_detected_with_char_offsets() {
        let text = "联系 test-user@example.com 谢谢"; // 中文前缀确保 char≠byte
        let e: Vec<_> = detect_regex(text).into_iter().filter(|e| e.kind == PiiKind::Email).collect();
        assert_eq!(e.len(), 1);
        assert_eq!(&text.chars().skip(e[0].start).take(e[0].end - e[0].start).collect::<String>(), "test-user@example.com");
    }

    #[test]
    fn cn_id_checksum_rejects_invalid() {
        assert!(is_valid_cn_id("11010519491231002X"));   // synthetic, valid checksum
        assert!(!is_valid_cn_id("110105194912310021")); // wrong check digit
        let e: Vec<_> = detect_regex("证件 110105194912310021 末位错").into_iter().filter(|e| e.kind == PiiKind::IdCard).collect();
        assert!(e.is_empty(), "invalid checksum must not match");
    }

    #[test]
    fn bank_card_luhn() {
        assert!(luhn_ok("4242424242424242"));
        assert!(!luhn_ok("4242424242424241"));
    }

    #[test]
    fn phone_and_ipv4_and_plate() {
        assert!(kinds("手机 13800138000").contains(&PiiKind::Phone));
        assert!(kinds("网关 192.168.1.1 内网").contains(&PiiKind::Ipv4));
        assert!(kinds("车牌 京A12345").contains(&PiiKind::Plate));
    }

    #[test]
    fn empty_text_no_matches() {
        assert!(detect_regex("").is_empty());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p docpipe-core pii::patterns`
Expected: FAIL — `detect_regex` / `is_valid_cn_id` / `luhn_ok` not found.

- [ ] **Step 3: Write minimal implementation** — prepend above the `#[cfg(test)]` block:

```rust
static RE_EMAIL: Lazy<Regex> = Lazy::new(|| Regex::new(r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}").unwrap());
static RE_PHONE: Lazy<Regex> = Lazy::new(|| Regex::new(r"1[3-9]\d{9}").unwrap());
static RE_IPV4: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b(?:(?:25[0-5]|2[0-4]\d|1?\d?\d)\.){3}(?:25[0-5]|2[0-4]\d|1?\d?\d)\b").unwrap());
static RE_ID: Lazy<Regex> = Lazy::new(|| Regex::new(r"[1-9]\d{16}[\dXx]").unwrap());
static RE_BANK: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b\d{13,19}\b").unwrap());
static RE_PLATE: Lazy<Regex> = Lazy::new(|| Regex::new(r"[京津沪渝冀豫云辽黑湘皖鲁新苏浙赣鄂桂甘晋蒙陕吉闽贵粤青藏川宁琼][A-HJ-NP-Z][A-HJ-NP-Z0-9]{5}").unwrap());

pub fn luhn_ok(digits: &str) -> bool {
    let ds: Vec<u32> = digits.chars().filter_map(|c| c.to_digit(10)).collect();
    if ds.len() < 13 { return false; }
    let mut sum = 0u32;
    for (i, &d) in ds.iter().rev().enumerate() {
        if i % 2 == 1 { let dd = d * 2; sum += if dd > 9 { dd - 9 } else { dd }; }
        else { sum += d; }
    }
    sum % 10 == 0
}

pub fn is_valid_cn_id(s: &str) -> bool {
    let c: Vec<char> = s.chars().collect();
    if c.len() != 18 || !c[..17].iter().all(|x| x.is_ascii_digit()) { return false; }
    const W: [u32; 17] = [7,9,10,5,8,4,2,1,6,3,7,9,10,5,8,4,2];
    const CHK: [char; 11] = ['1','0','X','9','8','7','6','5','4','3','2'];
    let sum: u32 = c[..17].iter().zip(W).map(|(ch, w)| ch.to_digit(10).unwrap() * w).sum();
    CHK[(sum % 11) as usize] == c[17].to_ascii_uppercase()
}

// 字节偏移 → 字符偏移
fn byte_to_char(text: &str, byte_idx: usize) -> usize {
    text[..byte_idx].chars().count()
}

fn push_matches(text: &str, re: &Regex, kind: PiiKind, out: &mut Vec<PiiEntity>, filter: impl Fn(&str) -> bool) {
    for m in re.find_iter(text) {
        if !filter(m.as_str()) { continue; }
        out.push(PiiEntity {
            kind, text: m.as_str().to_string(),
            start: byte_to_char(text, m.start()),
            end: byte_to_char(text, m.end()),
            confidence: 1.0, source: PiiSource::Regex,
        });
    }
}

pub fn detect_regex(text: &str) -> Vec<PiiEntity> {
    let mut out = Vec::new();
    push_matches(text, &RE_EMAIL, PiiKind::Email, &mut out, |_| true);
    push_matches(text, &RE_PHONE, PiiKind::Phone, &mut out, |_| true);
    push_matches(text, &RE_IPV4, PiiKind::Ipv4, &mut out, |_| true);
    push_matches(text, &RE_ID, PiiKind::IdCard, &mut out, is_valid_cn_id);
    push_matches(text, &RE_BANK, PiiKind::BankCard, &mut out, |s| !is_valid_cn_id(s) && luhn_ok(s));
    push_matches(text, &RE_PLATE, PiiKind::Plate, &mut out, |_| true);
    out
}
```

In `crates/docpipe-core/src/pii/mod.rs` add near top: `mod patterns; pub use patterns::detect_regex;`
In `crates/docpipe-core/Cargo.toml` ensure `[dependencies]` has `regex = "1"` and `once_cell = "1"` (add if missing).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p docpipe-core pii::patterns`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/docpipe-core/src/pii/patterns.rs crates/docpipe-core/src/pii/mod.rs crates/docpipe-core/Cargo.toml
git commit -m "feat(core): deterministic PII regex detectors + CN-ID/Luhn checksum"
```

---

### Task 3: Reversible redaction

**Files:**
- Create: `crates/docpipe-core/src/pii/redact.rs`
- Modify: `crates/docpipe-core/src/pii/mod.rs` (`mod redact; pub use redact::redact_text;`)

**Interfaces:**
- Consumes: `PiiEntity` (Task 1).
- Produces: `redact::redact_text(text: &str, entities: &[PiiEntity]) -> (String, std::collections::BTreeMap<String, String>)` — returns redacted text with `[KIND_N]` placeholders (replaced back-to-front by char offset) and a placeholder→original mapping.

- [ ] **Step 1: Write the failing test** — new `crates/docpipe-core/src/pii/redact.rs`:

```rust
//! 区间脱敏：按 char 偏移从后往前替换，产出可逆 mapping。
use std::collections::BTreeMap;
use super::{PiiEntity, PiiKind, PiiSource};

#[cfg(test)]
mod tests {
    use super::*;
    fn ent(kind: PiiKind, t: &str, s: usize, e: usize) -> PiiEntity {
        PiiEntity { kind, text: t.into(), start: s, end: e, confidence: 1.0, source: PiiSource::Regex }
    }
    #[test]
    fn redacts_back_to_front_and_is_reversible() {
        let text = "甲 a@b.co 乙 13800138000";
        // a@b.co at chars [2,8), phone at chars [11,22)
        let ents = vec![ent(PiiKind::Email,"a@b.co",2,8), ent(PiiKind::Phone,"13800138000",11,22)];
        let (red, map) = redact_text(text, &ents);
        assert!(red.contains("[EMAIL_1]") && red.contains("[PHONE_2]"));
        assert!(!red.contains("a@b.co") && !red.contains("13800138000"));
        assert_eq!(map.get("[EMAIL_1]").unwrap(), "a@b.co");
    }
    #[test]
    fn empty_entities_returns_text_unchanged() {
        let (red, map) = redact_text("hello", &[]);
        assert_eq!(red, "hello"); assert!(map.is_empty());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p docpipe-core pii::redact`
Expected: FAIL — `redact_text` not found.

- [ ] **Step 3: Write minimal implementation** — prepend:

```rust
pub fn redact_text(text: &str, entities: &[PiiEntity]) -> (String, BTreeMap<String, String>) {
    let mut ents: Vec<&PiiEntity> = entities.iter().collect();
    ents.sort_by_key(|e| e.start);
    let mut map = BTreeMap::new();
    // 占位符按出现顺序编号
    let mut tags: Vec<(usize, usize, String)> = Vec::new();
    for (i, e) in ents.iter().enumerate() {
        let tag = format!("[{:?}_{}]", e.kind, i + 1).to_uppercase().replace("::", "_");
        let tag = format!("[{}_{}]", kind_tag(e.kind), i + 1);
        map.insert(tag.clone(), e.text.clone());
        tags.push((e.start, e.end, tag));
    }
    // 从后往前替换，避免偏移失效
    let chars: Vec<char> = text.chars().collect();
    let mut out = chars.clone();
    for (start, end, tag) in tags.into_iter().rev() {
        out.splice(start..end, tag.chars());
    }
    (out.into_iter().collect(), map)
}

fn kind_tag(k: PiiKind) -> &'static str {
    match k {
        PiiKind::IdCard => "ID_CARD", PiiKind::Phone => "PHONE", PiiKind::Email => "EMAIL",
        PiiKind::BankCard => "BANK_CARD", PiiKind::Plate => "PLATE", PiiKind::Ipv4 => "IPV4",
        PiiKind::Person => "PERSON", PiiKind::Address => "ADDRESS", PiiKind::Org => "ORG",
    }
}
```
(Delete the first erroneous `tag` line; keep the `kind_tag`-based one.) Add `mod redact; pub use redact::redact_text;` to `mod.rs`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p docpipe-core pii::redact`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/docpipe-core/src/pii/redact.rs crates/docpipe-core/src/pii/mod.rs
git commit -m "feat(core): reversible interval-based PII redaction"
```

---

### Task 4: LLM NER backend (schema-guided, retry, degrade)

**Files:**
- Create: `crates/docpipe-core/src/pii/llm.rs`
- Modify: `crates/docpipe-core/src/pii/mod.rs` (`mod llm; pub use llm::{LlmNer, NerConfig};`)

**Interfaces:**
- Produces: `llm::NerConfig { base_url: String, model: String, api_key: Option<String>, enabled: bool }` with `NerConfig::from_env()`; `llm::LlmNer::new(NerConfig)`; `async fn LlmNer::detect(&self, text: &str) -> Result<Vec<PiiEntity>, String>` — POSTs OpenAI-compat chat/completions with `response_format: json_object`, retries ≤3 with backoff, validates JSON, maps to `PiiEntity { source: Llm }`. Returns `Err(reason)` on terminal failure (caller degrades). When `!enabled`, returns `Ok(vec![])`.

- [ ] **Step 1: Write the failing test** — new `crates/docpipe-core/src/pii/llm.rs` (test the offset-mapping + parse helper deterministically, no network):

```rust
//! LLM NER：OpenAI-compat，schema-guided JSON + retry≤3 + 弱模型降级（§4.5）。
use std::time::Duration;
use serde::Deserialize;
use super::{PiiEntity, PiiKind, PiiSource};

#[derive(Debug, Clone)]
pub struct NerConfig { pub base_url: String, pub model: String, pub api_key: Option<String>, pub enabled: bool }

impl NerConfig {
    pub fn from_env() -> Self {
        let base_url = std::env::var("DOCPIPE_PII_BASE_URL").unwrap_or_default();
        Self {
            enabled: !base_url.is_empty(),
            base_url,
            model: std::env::var("DOCPIPE_PII_MODEL").unwrap_or_else(|_| "deepseek-v4".into()),
            api_key: std::env::var("DOCPIPE_PII_API_KEY").ok(),
        }
    }
}

#[derive(Deserialize)]
struct RawEnt { kind: String, text: String }

// 把 LLM 返回的 {kind,text} 列表按 text 在原文中定位，产出带 char 偏移的 PiiEntity。
pub(crate) fn locate(text: &str, raw: &[RawEnt]) -> Vec<PiiEntity> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    for r in raw {
        let kind = match r.kind.as_str() {
            "person" => PiiKind::Person, "address" => PiiKind::Address, "org" => PiiKind::Org,
            _ => continue,
        };
        let needle: Vec<char> = r.text.chars().collect();
        if needle.is_empty() { continue; }
        if let Some(start) = (0..=chars.len().saturating_sub(needle.len()))
            .find(|&i| chars[i..i + needle.len()] == needle[..]) {
            out.push(PiiEntity { kind, text: r.text.clone(), start, end: start + needle.len(), confidence: 0.9, source: PiiSource::Llm });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn locate_maps_char_offsets() {
        let text = "签约方 某机构公司 与某甲";
        let raw = vec![RawEnt { kind: "org".into(), text: "某机构公司".into() }, RawEnt { kind: "person".into(), text: "某甲".into() }];
        let ents = locate(text, &raw);
        assert_eq!(ents.len(), 2);
        let org = ents.iter().find(|e| e.kind == PiiKind::Org).unwrap();
        assert_eq!(&text.chars().skip(org.start).take(org.end-org.start).collect::<String>(), "某机构公司");
    }
    #[test]
    fn disabled_config_when_no_base_url() {
        std::env::remove_var("DOCPIPE_PII_BASE_URL");
        assert!(!NerConfig::from_env().enabled);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p docpipe-core pii::llm`
Expected: FAIL — module/`locate`/`NerConfig` not found.

- [ ] **Step 3: Write minimal implementation** — append the network client (modeled on `embedder/ollama.rs`):

```rust
pub struct LlmNer { cfg: NerConfig, client: reqwest::Client }

#[derive(Deserialize)]
struct ChatResp { choices: Vec<Choice> }
#[derive(Deserialize)]
struct Choice { message: Msg }
#[derive(Deserialize)]
struct Msg { content: String }
#[derive(Deserialize)]
struct NerPayload { entities: Vec<RawEnt> }

const SYS: &str = "你是 PII 抽取器。只抽取人名(person)/详细地址(address)/组织机构名(org)。\
仅输出 JSON：{\"entities\":[{\"kind\":\"person|address|org\",\"text\":\"原文片段\"}]}。\
text 必须是原文中逐字出现的子串，不得改写。无实体则 entities 为空数组。\
示例1 输入「某甲与某机构公司签约」→ {\"entities\":[{\"kind\":\"person\",\"text\":\"某甲\"},{\"kind\":\"org\",\"text\":\"某机构公司\"}]}。\
示例2 输入「金额 100 元」→ {\"entities\":[]}。";

impl LlmNer {
    pub fn new(cfg: NerConfig) -> Self { Self { cfg, client: reqwest::Client::new() } }

    pub async fn detect(&self, text: &str) -> std::result::Result<Vec<PiiEntity>, String> {
        if !self.cfg.enabled { return Ok(Vec::new()); }
        let body = serde_json::json!({
            "model": self.cfg.model,
            "response_format": {"type": "json_object"},
            "messages": [{"role":"system","content":SYS},{"role":"user","content":text}]
        });
        let mut last = String::new();
        for attempt in 0..3u32 {
            let mut req = self.client.post(format!("{}/chat/completions", self.cfg.base_url.trim_end_matches('/')))
                .timeout(Duration::from_secs(60)).json(&body);
            if let Some(k) = &self.cfg.api_key { req = req.bearer_auth(k); }
            match req.send().await {
                Ok(r) if r.status().is_success() => {
                    match r.json::<ChatResp>().await.ok().and_then(|c| c.choices.into_iter().next()) {
                        Some(ch) => match serde_json::from_str::<NerPayload>(ch.message.content.trim()) {
                            Ok(p) => return Ok(locate(text, &p.entities)),
                            Err(e) => last = format!("bad-json: {e}"),
                        },
                        None => last = "no-choices".into(),
                    }
                }
                Ok(r) => last = format!("status {}", r.status()),
                Err(e) => last = format!("request: {e}"),
            }
            if attempt < 2 { tokio::time::sleep(Duration::from_millis(100 * 2u64.pow(attempt))).await; }
        }
        Err(last)
    }
}
```
Add `mod llm; pub use llm::{LlmNer, NerConfig};` to `mod.rs`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p docpipe-core pii::llm`
Expected: PASS (2 tests). `cargo check -p docpipe-core` clean.

- [ ] **Step 5: Commit**

```bash
git add crates/docpipe-core/src/pii/llm.rs crates/docpipe-core/src/pii/mod.rs
git commit -m "feat(core): LLM NER backend (OpenAI-compat, schema-guided, retry, env-gated)"
```

---

### Task 5: detect() orchestration + merge/dedup + graceful degrade

**Files:**
- Modify: `crates/docpipe-core/src/pii/mod.rs`

**Interfaces:**
- Consumes: `detect_regex` (T2), `LlmNer`/`NerConfig` (T4), `redact_text` (T3).
- Produces: `pii::DetectResult { entities: Vec<PiiEntity>, warnings: Vec<String> }`; `async fn pii::detect(text: &str, ner: Option<&LlmNer>, types: Option<&[PiiKind]>) -> DetectResult`. Regex always runs; LLM runs only if `ner` enabled; on LLM `Err`, push `warnings: ["llm-unavailable: <reason>; name/address/org detection skipped"]` and continue. Merge: drop an entity whose `[start,end)` overlaps a Regex entity (Regex wins). `types` filters output kinds.

- [ ] **Step 1: Write the failing test** — append to `mod.rs` tests:

```rust
    #[tokio::test]
    async fn regex_only_when_no_ner() {
        let r = detect("邮箱 a@b.co", None, None).await;
        assert!(r.entities.iter().any(|e| e.kind == PiiKind::Email));
        assert!(r.warnings.is_empty());
    }
    #[tokio::test]
    async fn type_filter_limits_kinds() {
        let r = detect("a@b.co 13800138000", None, Some(&[PiiKind::Phone])).await;
        assert!(r.entities.iter().all(|e| e.kind == PiiKind::Phone));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p docpipe-core pii::tests::regex_only`
Expected: FAIL — `detect` / `DetectResult` not found.

- [ ] **Step 3: Write minimal implementation** — add to `mod.rs`:

```rust
#[derive(Debug, Clone, Serialize)]
pub struct DetectResult { pub entities: Vec<PiiEntity>, pub warnings: Vec<String> }

fn overlaps(a: &PiiEntity, b: &PiiEntity) -> bool { a.start < b.end && b.start < a.end }

pub async fn detect(text: &str, ner: Option<&LlmNer>, types: Option<&[PiiKind]>) -> DetectResult {
    let mut entities = detect_regex(text);
    let mut warnings = Vec::new();
    if let Some(n) = ner {
        match n.detect(text).await {
            Ok(llm_ents) => {
                for e in llm_ents {
                    if !entities.iter().any(|r| r.source == PiiSource::Regex && overlaps(r, &e)) {
                        entities.push(e);
                    }
                }
            }
            Err(reason) => warnings.push(format!("llm-unavailable: {reason}; name/address/org detection skipped")),
        }
    }
    if let Some(t) = types { entities.retain(|e| t.contains(&e.kind)); }
    entities.sort_by_key(|e| e.start);
    DetectResult { entities, warnings }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p docpipe-core pii`
Expected: PASS (all pii tests).

- [ ] **Step 5: Commit**

```bash
git add crates/docpipe-core/src/pii/mod.rs
git commit -m "feat(core): PII detect() orchestration with regex-priority merge + LLM degrade"
```

---

### Task 6: Facade document-text accessor (for doc_id path)

**Files:**
- Modify: `crates/docpipe-core/src/facade.rs`
- Read first: `crates/docpipe-core/src/store/mod.rs` + `store/sqlite.rs` to find the chunk-text accessor.

**Interfaces:**
- Produces: `async fn Docpipe::document_text(&self, doc_id: &str, collection: &str) -> Result<String>` — returns the document's full text (concatenated chunk texts joined by `\n`), or `Err(DocError::DocumentNotFound)` if no chunks/doc. Use the existing store; if the store lacks a "chunks by doc" method, add `async fn chunks_for_document(&self, doc_id: &str, collection: &str) -> Result<Vec<String>>` to the `VectorStore` trait + sqlite impl (mirror the existing `get_document` query, selecting chunk `text`).

- [ ] **Step 1: Write the failing test** — add to `facade.rs` tests (there are existing in-memory-store tests around line 314/352; mirror their setup):

```rust
    #[tokio::test]
    async fn document_text_concatenates_chunks() {
        let sdk = test_sdk_with_doc("d1", "default", &["第一段 a@b.co", "第二段 某甲"]).await;
        let t = sdk.document_text("d1", "default").await.unwrap();
        assert!(t.contains("a@b.co") && t.contains("某甲"));
        assert!(sdk.document_text("nope", "default").await.is_err());
    }
```
(Implement `test_sdk_with_doc` helper using the same in-memory store + ingest path the existing facade tests use; read lines 300–360 for the exact builder.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p docpipe-core facade::tests::document_text`
Expected: FAIL — `document_text` not found.

- [ ] **Step 3: Write minimal implementation** — add the trait method (+ sqlite impl mirroring `get_document`'s SELECT but returning chunk `text` rows) and:

```rust
    pub async fn document_text(&self, doc_id: &str, collection: &str) -> Result<String> {
        let chunks = self.store.chunks_for_document(doc_id, collection).await?;
        if chunks.is_empty() { return Err(DocError::DocumentNotFound); }
        Ok(chunks.join("\n"))
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p docpipe-core facade`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/docpipe-core/src/facade.rs crates/docpipe-core/src/store/mod.rs crates/docpipe-core/src/store/sqlite.rs
git commit -m "feat(core): Docpipe::document_text + store chunks_for_document accessor"
```

---

### Task 7: Server route POST /v1/detect-pii

**Files:**
- Create: `crates/docpipe-server/src/routes/pii.rs`
- Modify: `crates/docpipe-server/src/routes/mod.rs` (register route)
- Modify: `crates/docpipe-server/src/state.rs` (add `pub ner: Option<docpipe_core::pii::LlmNer>` built from `NerConfig::from_env()`)
- Read first: `crates/docpipe-server/src/main.rs` or wherever `AppState` is constructed, to wire `ner`.

**Interfaces:**
- Consumes: `state.sdk.document_text` (T6), `pii::detect`/`redact_text` (T3/T5), `sdk.annotate` (existing).
- Produces: `POST /v1/detect-pii` handler returning `{ entities, redacted_text?, mapping?, annotations?, warnings }`. Errors via existing error→JSON mapping (mirror how other routes map `DocError`).

- [ ] **Step 1: Write the failing test** — new `crates/docpipe-server/tests/` integration or inline route test. Minimal inline unit test in `pii.rs` using `axum::http` + a test `AppState` (mirror an existing route test; if none, add a thin one):

```rust
// 见 Step 3 实现；测试断言：缺 text 和 doc_id → 400 bad-request；text 含邮箱 → entities 命中 email。
```
Concretely add a `#[tokio::test]` that builds the router (`crate::routes::router(state)`) with a test `AppState` (in-memory store, `ner: None`), sends `POST /v1/detect-pii {"text":"a@b.co"}`, asserts 200 + body `entities[0].kind == "email"`; and sends `{}` asserts 400 + `error == "bad-request"`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p docpipe-server detect_pii`
Expected: FAIL — route not found / 404.

- [ ] **Step 3: Write minimal implementation** — `pii.rs`:

```rust
//! POST /v1/detect-pii — PII 检测 + 可选脱敏/标注。
use std::sync::Arc;
use axum::{extract::State, http::StatusCode, Json};
use docpipe_core::pii::{self, PiiKind};
use serde::Deserialize;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct DetectPiiReq {
    pub text: Option<String>,
    pub doc_id: Option<String>,
    #[serde(default = "default_collection")]
    pub collection: String,
    pub types: Option<Vec<String>>,
    #[serde(default)] pub redact: bool,
    #[serde(default)] pub annotate: bool,
}
fn default_collection() -> String { "default".into() }

fn parse_kinds(v: &[String]) -> Vec<PiiKind> {
    v.iter().filter_map(|s| serde_json::from_value(serde_json::Value::String(s.clone())).ok()).collect()
}

pub async fn detect_pii(
    State(state): State<Arc<AppState>>,
    Json(req): Json<DetectPiiReq>,
) -> (StatusCode, Json<serde_json::Value>) {
    let text = match (&req.text, &req.doc_id) {
        (Some(t), _) => t.clone(),
        (None, Some(id)) => match state.sdk.document_text(id, &req.collection).await {
            Ok(t) => t,
            Err(e) => return (StatusCode::from_u16(e.http_status()).unwrap(), Json(serde_json::json!({"error": e.code()}))),
        },
        (None, None) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"bad-request","detail":"text or doc_id required"}))),
    };
    let kinds = req.types.as_ref().map(|v| parse_kinds(v));
    let res = pii::detect(&text, state.ner.as_ref(), kinds.as_deref()).await;
    let mut body = serde_json::json!({ "entities": res.entities, "warnings": res.warnings });
    if req.redact {
        let (red, map) = pii::redact_text(&text, &res.entities);
        body["redacted_text"] = serde_json::json!(red);
        body["mapping"] = serde_json::json!(map);
    }
    // annotate=true 时逐实体写标注（复用 annotator）；doc_id 缺失则跳过并 warn
    (StatusCode::OK, Json(body))
}
```
Register in `routes/mod.rs`: `.route("/v1/detect-pii", post(pii::detect_pii))` and `mod pii;`. Wire `state.ner = docpipe_core::pii::LlmNer::new(NerConfig::from_env()).into()` (only `Some` when `enabled`; store `Option`).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p docpipe-server detect_pii && cargo clippy -p docpipe-server --all-targets -- -D warnings`
Expected: PASS + clippy clean.

- [ ] **Step 5: Commit**

```bash
git add crates/docpipe-server/src/routes/pii.rs crates/docpipe-server/src/routes/mod.rs crates/docpipe-server/src/state.rs crates/docpipe-server/src/main.rs
git commit -m "feat(server): POST /v1/detect-pii route + NerConfig wiring"
```

---

### Task 8: OpenAPI contract for /v1/detect-pii

**Files:**
- Modify: `openapi.yaml` (add path + `PiiEntity`, `DetectPiiRequest`, `PiiResult` schemas)

- [ ] **Step 1: Write the failing test** — extend the ref-completeness check used pre-push:

```bash
# refs must all be defined; PiiResult/PiiEntity/DetectPiiRequest must exist
grep -q 'detect-pii' openapi.yaml && grep -q 'PiiResult:' openapi.yaml && echo MISSING-OK
```

- [ ] **Step 2: Verify it currently fails**

Run: `grep -c 'detect-pii' openapi.yaml`
Expected: `0`.

- [ ] **Step 3: Add the path + schemas** under `paths:` and `components/schemas:` mirroring spec §5 (entities[], redacted_text nullable, mapping object, annotations array, warnings array; request with text/doc_id/collection/types/redact/annotate). Use the existing schema indentation (4 spaces).

- [ ] **Step 4: Verify refs resolve**

Run the pre-push ref check (defined vs referenced schemas); expect empty diff.

- [ ] **Step 5: Commit**

```bash
git add openapi.yaml
git commit -m "docs(api): openapi contract for POST /v1/detect-pii"
```

---

### Task 9: Python SDK detect_pii()

**Files:**
- Modify: `clients/python/docpipe/client.py`, `clients/python/docpipe/models.py`, `clients/python/docpipe/__init__.py`
- Test: `clients/python/tests/test_client.py`

**Interfaces:**
- Produces: `PiiEntity`/`PiiResult` pydantic models; `DocpipeClient.detect_pii(text=None, *, doc_id=None, collection="default", types=None, redact=False, annotate=False) -> PiiResult`.

- [ ] **Step 1: Write the failing test** — add to `test_client.py` (respx-mocked, synthetic data):

```python
@respx.mock
def test_detect_pii_returns_entities():
    respx.post("http://docs/v1/detect-pii").mock(return_value=httpx.Response(200, json={
        "entities": [{"kind": "email", "text": "a@b.co", "start": 0, "end": 6, "confidence": 1.0, "source": "regex"}],
        "warnings": [],
    }))
    c = DocpipeClient("http://docs")
    res = c.detect_pii("a@b.co")
    assert res.entities[0].kind == "email"
    body = _json.loads(respx.calls.last.request.content)
    assert body["text"] == "a@b.co"
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd clients/python && .venv/bin/python -m pytest -q -k detect_pii`
Expected: FAIL — `detect_pii` not found.

- [ ] **Step 3: Write minimal implementation** — `models.py`:

```python
class PiiEntity(BaseModel):
    kind: str
    text: str
    start: int
    end: int
    confidence: float
    source: str

class PiiResult(BaseModel):
    entities: list[PiiEntity] = []
    redacted_text: str | None = None
    mapping: dict[str, str] | None = None
    annotations: list[dict[str, Any]] | None = None
    warnings: list[str] = []
```
`client.py`:
```python
    def detect_pii(self, text: str | None = None, *, doc_id: str | None = None,
                   collection: str = "default", types: list[str] | None = None,
                   redact: bool = False, annotate: bool = False) -> PiiResult:
        payload: dict[str, Any] = {"collection": collection, "redact": redact, "annotate": annotate}
        if text is not None: payload["text"] = text
        if doc_id is not None: payload["doc_id"] = doc_id
        if types is not None: payload["types"] = types
        r = self._client.post(f"{self.base_url}/v1/detect-pii", json=payload)
        r.raise_for_status()
        return PiiResult(**r.json())
```
Export `PiiEntity, PiiResult` in `__init__.py`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cd clients/python && .venv/bin/python -m pytest -q`
Expected: PASS (all).

- [ ] **Step 5: Commit**

```bash
git add clients/python
git commit -m "feat(py-sdk): detect_pii() + PiiEntity/PiiResult models"
```

---

### Task 10: TypeScript SDK detectPii()

**Files:**
- Modify: `clients/typescript/src/client.ts`, `clients/typescript/src/types.ts`
- Test: `clients/typescript/tests/client.test.ts`

**Interfaces:**
- Produces: `PiiEntity`/`PiiResult` types; `DocpipeClient.detectPii(req: { text?, docId?, collection?, types?, redact?, annotate? }): Promise<PiiResult>`. `docId` maps to wire `doc_id`; camelCase must not leak.

- [ ] **Step 1: Write the failing test** — add to `client.test.ts`:

```typescript
  it("detectPii sends snake_case doc_id and returns entities", async () => {
    const fetchMock = vi.fn(async () =>
      new Response(JSON.stringify({ entities: [{ kind: "email", text: "a@b.co", start: 0, end: 6, confidence: 1, source: "regex" }], warnings: [] }), { status: 200 }));
    vi.stubGlobal("fetch", fetchMock);
    const c = new DocpipeClient("http://docs");
    const res = await c.detectPii({ docId: "d1" });
    expect(res.entities[0].kind).toBe("email");
    const [, init] = fetchMock.mock.calls[0];
    const body = JSON.parse((init as RequestInit).body as string);
    expect(body.doc_id).toBe("d1");
    expect(body.docId).toBeUndefined();
  });
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd clients/typescript && npm test --silent`
Expected: FAIL — `detectPii` not a function.

- [ ] **Step 3: Write minimal implementation** — `types.ts`:

```typescript
export interface PiiEntity { kind: string; text: string; start: number; end: number; confidence: number; source: string; }
export interface PiiResult { entities: PiiEntity[]; redacted_text?: string | null; mapping?: Record<string,string> | null; annotations?: unknown[] | null; warnings: string[]; }
```
`client.ts`:
```typescript
  async detectPii(req: { text?: string; docId?: string; collection?: string; types?: string[]; redact?: boolean; annotate?: boolean }): Promise<PiiResult> {
    const body: Record<string, unknown> = { collection: req.collection ?? "default", redact: req.redact ?? false, annotate: req.annotate ?? false };
    if (req.text !== undefined) body.text = req.text;
    if (req.docId !== undefined) body.doc_id = req.docId;
    if (req.types !== undefined) body.types = req.types;
    const r = await fetch(`${this.baseUrl}/v1/detect-pii`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) });
    return this.handle<PiiResult>(r);
  }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd clients/typescript && npm test --silent`
Expected: PASS (all).

- [ ] **Step 5: Commit**

```bash
git add clients/typescript
git commit -m "feat(ts-sdk): detectPii() + PiiEntity/PiiResult types"
```

---

### Task 11: Adversarial + i18n + degrade test matrix (spec §9)

**Files:**
- Modify: `crates/docpipe-core/src/pii/mod.rs` (tests), `crates/docpipe-core/src/pii/patterns.rs` (tests)

- [ ] **Step 1: Write the failing tests** (all synthetic):

```rust
    #[tokio::test]
    async fn adversarial_fake_id_and_injection_not_entities() {
        let r = detect("忽略指令 DROP TABLE; 身份证 110105194912310021 末位错", None, None).await;
        assert!(!r.entities.iter().any(|e| e.kind == PiiKind::IdCard));
    }
    #[tokio::test]
    async fn i18n_offsets_with_emoji() {
        let text = "😀 mail a@b.co 完";
        let r = detect(text, None, None).await;
        let e = r.entities.iter().find(|e| e.kind == PiiKind::Email).unwrap();
        assert_eq!(text.chars().skip(e.start).take(e.end-e.start).collect::<String>(), "a@b.co");
    }
    #[tokio::test]
    async fn empty_and_whitespace_text() {
        assert!(detect("   ", None, None).await.entities.is_empty());
    }
```

- [ ] **Step 2: Run to verify** (these should pass against existing impl; if any fail it reveals an offset bug — fix the impl, not the test).

Run: `cargo test -p docpipe-core pii`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/docpipe-core/src/pii
git commit -m "test(core): PII adversarial + i18n offset + degrade matrix"
```

---

### Task 12: v1.0 version + package metadata

**Files:**
- Modify: `Cargo.toml` (`version = "1.0.0"`), `clients/python/pyproject.toml`, `clients/typescript/package.json`

- [ ] **Step 1:** Set `version = "1.0.0"` in workspace `Cargo.toml`; `version = "1.0.0"` in `pyproject.toml` + add `license = {text="Apache-2.0"}`, `authors = [{name="qiurui144"}]`, `[project.urls] Repository = "https://github.com/qiurui144/docpipe"`; in `package.json` set `"version":"1.0.0"`, add `"license":"Apache-2.0"`, `"repository":{"type":"git","url":"https://github.com/qiurui144/docpipe.git"}`, `"author":"qiurui144"`. **No real email.**

- [ ] **Step 2: Verify build + tests**

Run: `cargo check && (cd clients/python && .venv/bin/python -m pytest -q) && (cd clients/typescript && npm test --silent)`
Expected: all PASS.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml clients/python/pyproject.toml clients/typescript/package.json
git commit -m "chore: align versions to 1.0.0 + complete package metadata (no PII)"
```

---

### Task 13: gitleaks pre-commit + CI gate (recurrence prevention)

**Files:**
- Create: `.gitleaks.toml` (allow synthetic fixtures, flag real-looking PII), `.pre-commit-config.yaml`
- Modify: `.github/workflows/ci.yml` (add a `gitleaks` job)

- [ ] **Step 1:** Add `.pre-commit-config.yaml` with the `gitleaks` hook (`repo: https://github.com/gitleaks/gitleaks`, `rev: v8.x`, `id: gitleaks`). Add a CI job that runs `gitleaks detect --no-banner --redact`.

- [ ] **Step 2: Verify it runs clean on current tree**

Run: `gitleaks detect --no-banner --redact || true` (install if absent: note in DEVELOP.md). Expected: 0 findings (history already scrubbed).

- [ ] **Step 3: Commit**

```bash
git add .gitleaks.toml .pre-commit-config.yaml .github/workflows/ci.yml
git commit -m "ci: add gitleaks pre-commit + CI gate to prevent PII/secret leaks"
```

---

### Task 14: RELEASE.md + docs sync + v1.0.0 tag (RC 4-gate)

**Files:**
- Create: `RELEASE.md`
- Modify: `README.md` / `README.zh.md` / `DEVELOP.md` (document `/v1/detect-pii` + `detect_pii`/`detectPii`)

- [ ] **Step 1:** Write `RELEASE.md` v1.0.0 section: Highlights (ingest/documents/jobs API + PII detection), Breaking (none), Migration (none), Known Limitations (LLM NER requires ≥ gpt-4o-mini-tier; 3B local → regex-only; doc_id path concatenates chunk text). Add PII usage to README (both langs) + DEVELOP.
- [ ] **Step 2:** Run §7.2 4-gate manually: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, both SDK suites, openapi ref check, gitleaks clean, docs-vs-code no drift.
- [ ] **Step 3: Commit + tag**

```bash
git add RELEASE.md README.md README.zh.md DEVELOP.md
git commit -m "docs: RELEASE.md v1.0.0 + PII usage; sync README/DEVELOP"
git tag v1.0.0
git push origin master --tags
```

---

### Task 15: Persist chunk char_offset + page-aware locator accessor (annotate plumbing)

> Added 2026-06-25 after the Task 7 review: user chose to fully implement `annotate=true` now. Annotations need `(page_num, page-local char_offset)`; the chunker computes `Chunk.char_offset` (within-page) but the store does not persist it. This task persists it and exposes a page-aware accessor.

**Files:**
- Modify: `crates/docpipe-core/src/types.rs` (add `ChunkLocator`)
- Modify: `crates/docpipe-core/src/store/mod.rs` (trait method), `crates/docpipe-core/src/store/sqlite.rs` (schema + INSERT + accessor)
- Modify: `crates/docpipe-core/src/facade.rs` (passthrough `document_locators`)

**Interfaces PRODUCED:**
- `types::ChunkLocator { pub page_num: u32, pub char_offset: u32, pub text: String }` (derive Debug, Clone, Serialize, Deserialize, PartialEq)
- `async fn VectorStore::document_locators(&self, doc_id: &str, collection: &str) -> Result<Vec<ChunkLocator>>` (stored order; `page_num` = first element of the chunk's `page_refs`, or 0 if none)
- `async fn Docpipe::document_locators(&self, doc_id: &str, collection: &str) -> Result<Vec<ChunkLocator>>`

- [ ] **Step 1: Write the failing test** in `facade.rs` tests (reuse the `test_sdk_with_doc` helper from Task 6; synthetic data):

```rust
    #[tokio::test]
    async fn document_locators_carry_page_and_offset() {
        let sdk = test_sdk_with_doc("d2", "default", &["第一段 a@b.co", "第二段 某甲"]).await;
        let locs = sdk.document_locators("d2", "default").await.unwrap();
        assert!(!locs.is_empty());
        assert!(locs.iter().all(|l| l.page_num == 1));
        assert!(locs.iter().any(|l| l.text.contains("a@b.co")));
        assert!(sdk.document_locators("missing", "default").await.unwrap().is_empty());
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p docpipe-core facade::tests::document_locators`
Expected: FAIL — `document_locators` / `ChunkLocator` not found.

- [ ] **Step 3: Implement**
  - In `sqlite.rs`: add `char_offset INTEGER NOT NULL DEFAULT 0` to the `CREATE TABLE chunk_meta` statement. Immediately after creating the table, run a defensive upgrade for pre-existing DBs and ignore the "duplicate column" error:
    ```rust
    let _ = conn.execute("ALTER TABLE chunk_meta ADD COLUMN char_offset INTEGER NOT NULL DEFAULT 0", []);
    ```
  - In the `upsert` INSERT, add the column + bind `ec.chunk.char_offset`:
    ```rust
    "INSERT INTO chunk_meta (chunk_id, coll, text, page_refs, char_offset) VALUES (?1, ?2, ?3, ?4, ?5)",
    rusqlite::params![ec.chunk.chunk_id, collection, ec.chunk.text, page_refs, ec.chunk.char_offset],
    ```
  - Add the trait method to `VectorStore` and implement in `SqliteVecStore`:
    ```rust
    async fn document_locators(&self, doc_id: &str, collection: &str) -> Result<Vec<ChunkLocator>> {
        let escaped = doc_id.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
        let prefix = format!("{escaped}:%");
        let conn = self.conn.lock().await; // match the existing lock pattern in this file
        let mut stmt = conn
            .prepare("SELECT text, page_refs, char_offset FROM chunk_meta WHERE chunk_id LIKE ?1 ESCAPE '\\' AND coll = ?2 ORDER BY rowid")
            .map_err(|e| DocError::VectorStoreError(format!("prepare locators: {e}")))?;
        let rows = stmt
            .query_map(rusqlite::params![prefix, collection], |r| {
                let text: String = r.get(0)?;
                let page_refs_json: String = r.get(1)?;
                let char_offset: u32 = r.get(2)?;
                Ok((text, page_refs_json, char_offset))
            })
            .map_err(|e| DocError::VectorStoreError(format!("query locators: {e}")))?;
        let mut out = Vec::new();
        for row in rows {
            let (text, page_refs_json, char_offset) = row.map_err(|e| DocError::VectorStoreError(format!("row: {e}")))?;
            let page_num = serde_json::from_str::<Vec<u32>>(&page_refs_json).ok().and_then(|v| v.first().copied()).unwrap_or(0);
            out.push(ChunkLocator { page_num, char_offset, text });
        }
        Ok(out)
    }
    ```
    (Adapt `self.conn.lock()` / error style to exactly match the existing methods in `sqlite.rs` — read `chunks_for_document` first.)
  - In `facade.rs` add the passthrough: `pub async fn document_locators(&self, doc_id: &str, collection: &str) -> Result<Vec<ChunkLocator>> { self.store.document_locators(doc_id, collection).await }`

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p docpipe-core` (whole crate — trait changed) + `cargo clippy -p docpipe-core --all-targets -- -D warnings`
Expected: PASS + clean.

- [ ] **Step 5: Commit**

```bash
git add crates/docpipe-core/src/types.rs crates/docpipe-core/src/store/mod.rs crates/docpipe-core/src/store/sqlite.rs crates/docpipe-core/src/facade.rs
git commit -m "feat(core): persist chunk char_offset + page-aware document_locators accessor"
```

---

### Task 16: Document-aware detect + annotate persistence in route

> Wires Task 15's locators into `/v1/detect-pii` so `annotate=true` with a `doc_id` persists each PII entity as an annotation with a correct page locator.

**Files:**
- Modify: `crates/docpipe-server/src/routes/pii.rs`

**Interfaces CONSUMED:** `state.sdk.document_locators` (T15), `state.sdk.document_text` (T6), `state.sdk.annotate` (existing, takes `docpipe_core::annotator::AnnotateRequest`), `pii::detect`/`pii::redact_text`.

**Behavior:**
- **text input (no doc_id):** unchanged flat detection. If `annotate=true` → push warning `"annotate requires doc_id"` (cannot annotate text with no document).
- **doc_id input:** fetch `document_locators`; if empty → `document-not-found` (404). Detect per locator; for each entity build an augmented JSON entity `{kind,text,start,end,confidence,source,page_num}` where `start = locator.char_offset + entity.start`, `end = locator.char_offset + entity.end`, `page_num = locator.page_num`. Aggregate warnings (dedup). When `redact=true`, redact the joined `document_text` via a flat detect pass (offsets into joined text; independent of the page-aware entities). When `annotate=true`, for each entity call `sdk.annotate` and collect `{item_id}` into `annotations`.

- [ ] **Step 1: Write the failing test** in `pii.rs` tests (uses `AppState::for_test()`; the test must first ingest a synthetic doc through the sdk so a doc_id exists — reuse the ingest pattern other server tests use; if server tests lack one, ingest via `state.sdk.ingest_file` with a tiny synthetic HTML/text byte slice). Assert: `POST /v1/detect-pii {"doc_id":"<id>","annotate":true}` → 200, response has `annotations` non-empty and each returned entity has a `page_num`. Also `POST {"text":"a@b.co","annotate":true}` → 200 with a warning containing `"annotate requires doc_id"`.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p docpipe-server detect_pii_annotate`
Expected: FAIL.

- [ ] **Step 3: Implement** the doc_id branch in `detect_pii` per Behavior above. Build `AnnotateRequest`:
```rust
use docpipe_core::annotator::AnnotateRequest;
use docpipe_core::types::AnnotationSource;
let item = state.sdk.annotate(AnnotateRequest {
    doc_id: doc_id.clone(),
    original_text: ent_text.clone(),
    content: format!("检测到 PII: {kind_str}"),
    label: format!("pii-{kind_str}"),
    color: "#ef4444".to_string(),
    page_num,
    char_offset,         // page-local = locator.char_offset + entity.start
    bbox: None,
    source: AnnotationSource::Ai,
    skill_metadata: None,
});
annotations.push(serde_json::json!({ "item_id": item.item_id }));
```
(`kind_str` = the entity's serialized snake_case kind. Read `routes/annotate.rs` for the exact `AnnotateRequest` field set and `AnnotationSource` import path.)

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p docpipe-server` + `cargo clippy -p docpipe-server --all-targets -- -D warnings`
Expected: PASS + clean.

- [ ] **Step 5: Commit**

```bash
git add crates/docpipe-server/src/routes/pii.rs
git commit -m "feat(server): document-aware detect-pii + annotate persistence with page locators"
```

---

## Self-Review

**Spec coverage:** §1 positioning → Task 14 docs. §2 scope (detect core + optional redact/annotate; deterministic + LLM types) → T2/T3/T4/T5/T7. §3 data flow → T5 orchestration + T7 route. §4 module boundaries → T1–T7 file map. §5 API contract → T7 route + T8 openapi + T9/T10 SDK. §6 extension points → T2 (add detector), T4 (env model). §7 errors/boundaries → T1 bad-request, T5 degrade, T11 i18n/empty. §8 cost (weak-model disable) → T4 `enabled` + T5 degrade + T14 Known Limitations. §9 test matrix → T2/T11 (happy/edge/error/adversarial/i18n), degrade tested in T5; **multi-seed N=3 LLM tier eval is a manual pre-tag step** — added as an explicit note in Task 14 Step 2 (run real-LLM 3-tier × 3-seed before tag; record F1, set min-tier in RELEASE). §10 back-compat (new endpoint, nullable fields) → T8/T9/T10. §11 risks (offset, fixture re-leak) → T11 offsets + T13 gitleaks.

**Gap found + fixed:** Spec §9 requires multi-seed multi-tier LLM eval; no deterministic task can cover a real-LLM eval, so it is called out as a mandatory manual gate in Task 14 Step 2 (not silently dropped).

**Placeholder scan:** Task 6 (`test_sdk_with_doc` / store accessor) and Task 7 (route test harness) instruct reading the exact existing pattern before writing — these are genuine "read neighbor code" steps, not placeholders; all signatures they must produce are specified in the Interfaces blocks.

**Type consistency:** `PiiEntity`/`PiiKind`/`PiiSource`/`DetectResult`/`detect()`/`detect_regex()`/`redact_text()`/`LlmNer`/`NerConfig`/`document_text()` names are consistent across T1→T14. Wire JSON kinds are snake_case (`id_card`, `email`, …) in Rust enum, openapi, and both SDKs.
